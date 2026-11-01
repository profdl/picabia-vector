//! Geometry rows: modifiers that change the object's paths instead of placing copies
//! (`resample`, `trim`, `tidy`, `displace`).
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

mod displace;
#[cfg(test)]
mod displace_tests;
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
use crate::model::{Displace, ModifierKind, Resample, Tidy, Trim};

/// The most anchors the geometry rows make for one object, all its copies together.
pub const MAX_GEOMETRY_ANCHORS: usize = 200_000;

/// A geometry row's operation, its params clamped.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryOp {
    Resample(Resample),
    Trim(Trim),
    Tidy(Tidy),
    Displace(Displace),
}

impl GeometryOp {
    /// The row's name, for notes.
    pub fn label(&self) -> &'static str {
        match self {
            GeometryOp::Resample(_) => "Resample",
            GeometryOp::Trim(_) => "Trim",
            GeometryOp::Tidy(_) => "Tidy",
            GeometryOp::Displace(_) => "Displace",
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
        ModifierKind::Displace(p) => {
            let mut p = p.clone();
            p.amplitude = c.within("amplitude", p.amplitude, 0.0, 100.0, 10.0);
            p.scale = c.within("scale", p.scale, 0.01, 1.0, 0.05);
            p.octaves = whole(&mut c, "octaves", p.octaves, 1, 6);
            p.ripple_count = whole(&mut c, "rippleCount", p.ripple_count, 2, 12);
            p.seed = whole(&mut c, "seed", p.seed, 0, 999_999);
            p.progressive_amount = c.within("progressiveAmount", p.progressive_amount, 0.0, 1.0, 1.0);
            GeometryOp::Displace(p)
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
/// for each copy: copy.transform(t); shaper.copy(&mut copy);
/// ```
pub struct Shaper<'a> {
    rows: &'a [GeometryRow],
    /// The anchors one copy may hold after a row that adds anchors.
    budget: usize,
    /// How many copies the evaluation makes, and the index [`Shaper::copy`] gives the next one.
    copies: usize,
    next: usize,
    /// What went wrong, each said once.
    pub notes: Vec<String>,
}

impl<'a> Shaper<'a> {
    /// A shaper for `out`'s rows, the budget shared by its copies.
    pub fn new(out: &'a Output) -> Shaper<'a> {
        let copies = out.instances.len().max(1);
        Shaper { rows: &out.geometry, budget: MAX_GEOMETRY_ANCHORS / copies, copies, next: 0, notes: Vec::new() }
    }

    /// Whether there is anything to run.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Run the rows above the copy rows on `n`, the object before it is copied.
    pub fn source(&mut self, n: &mut Node) {
        self.run(n, RowAt::Source, None);
    }

    /// Run the rows below a copy row on `n`, one copy after its transform: the copy after the
    /// last one given (the first: 0).
    pub fn copy(&mut self, n: &mut Node) {
        self.copy_at(n, self.next);
    }

    /// Run the rows below a copy row on `n`, copy `index` of [`Output::instances`] after its
    /// transform (a displace varies by copy).
    pub fn copy_at(&mut self, n: &mut Node, index: usize) {
        self.next = index.saturating_add(1);
        self.run(n, RowAt::Copies, Some((index, self.copies)));
    }

    fn run(&mut self, n: &mut Node, at: RowAt, copy: Option<(usize, usize)>) {
        let rows = self.rows;
        for row in rows.iter().filter(|r| r.at == at) {
            self.row(n, row, copy);
        }
    }

    fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    /// One row on `n`, or none of it (over the budget, or a point that isn't finite).
    fn row(&mut self, n: &mut Node, row: &GeometryRow, copy: Option<(usize, usize)>) {
        let label = row.op.label();
        let (before, after) = (anchors(n, 0), projected(n, &row.op, 0));
        if after > self.budget && after > before {
            let total = MAX_GEOMETRY_ANCHORS;
            self.note(format!("{label} would make more than {total} anchors for this object and its copies; it is skipped"));
            return;
        }
        let field = field(n, &row.op, copy);
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
                // A displace moves anchors, never adds them.
                GeometryOp::Trim(_) | GeometryOp::Tidy(_) | GeometryOp::Displace(_) => sp.anchors.len(),
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
        NodeKind::Path { guide: true, .. } => {}
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

/// A displace's field for `n` (the radial centre is the mean of `n`'s anchors) and copy `copy`;
/// `None` for the other rows.
fn field(n: &Node, op: &GeometryOp, copy: Option<(usize, usize)>) -> Option<displace::Field> {
    field_from(anchor_sum(n, 0), op, copy)
}

fn field_from((sum, count): (Vec2, usize), op: &GeometryOp, copy: Option<(usize, usize)>) -> Option<displace::Field> {
    let GeometryOp::Displace(p) = op else { return None };
    let centre = if count > 0 { (sum / count as f64).to_point() } else { Point::ZERO };
    Some(displace::Field::new(p, centre, copy))
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
    let mut changed = false;
    let mut subpaths: Vec<SubPath> = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let new = match op {
            GeometryOp::Resample(p) => resample::resample(sp, p),
            GeometryOp::Tidy(p) => tidy::tidy(sp, p),
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
/// displace as on the object: its radial centre the mean of `path`'s anchors, no copy ramp).
pub fn shape_path(path: &PathData, op: &GeometryOp) -> PathData {
    let field = field_from(path_sum(path), op, None);
    shaped(path, op, field.as_ref()).unwrap_or_else(|| path.clone())
}
