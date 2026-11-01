//! Geometry rows: modifiers that change the object's paths instead of placing copies
//! (`resample`, `trim`, `tidy`).
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

mod pieces;
mod resample;
#[cfg(test)]
mod tests;
mod tidy;
mod trim;

use std::sync::Arc;

use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{PathData, SubPath};

use crate::eval::Output;
use crate::kinds::Clamp;
use crate::model::{ModifierKind, Resample, Tidy, Trim};

/// The most anchors the geometry rows make for one object, all its copies together.
pub const MAX_GEOMETRY_ANCHORS: usize = 200_000;

/// A geometry row's operation, its params clamped.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryOp {
    Resample(Resample),
    Trim(Trim),
    Tidy(Tidy),
}

impl GeometryOp {
    /// The row's name, for notes.
    pub fn label(&self) -> &'static str {
        match self {
            GeometryOp::Resample(_) => "Resample",
            GeometryOp::Trim(_) => "Trim",
            GeometryOp::Tidy(_) => "Tidy",
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
    /// What went wrong, each said once.
    pub notes: Vec<String>,
}

impl<'a> Shaper<'a> {
    /// A shaper for `out`'s rows, the budget shared by its copies.
    pub fn new(out: &'a Output) -> Shaper<'a> {
        let copies = out.instances.len().max(1);
        Shaper { rows: &out.geometry, budget: MAX_GEOMETRY_ANCHORS / copies, notes: Vec::new() }
    }

    /// Whether there is anything to run.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Run the rows above the copy rows on `n`, the object before it is copied.
    pub fn source(&mut self, n: &mut Node) {
        self.run(n, RowAt::Source);
    }

    /// Run the rows below a copy row on `n`, one copy after its transform.
    pub fn copy(&mut self, n: &mut Node) {
        self.run(n, RowAt::Copies);
    }

    fn run(&mut self, n: &mut Node, at: RowAt) {
        let rows = self.rows;
        for row in rows.iter().filter(|r| r.at == at) {
            self.row(n, row);
        }
    }

    fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    /// One row on `n`, or none of it (over the budget, or a point that isn't finite).
    fn row(&mut self, n: &mut Node, row: &GeometryRow) {
        let label = row.op.label();
        let (before, after) = (anchors(n, 0), projected(n, &row.op, 0));
        if after > self.budget && after > before {
            let total = MAX_GEOMETRY_ANCHORS;
            self.note(format!("{label} would make more than {total} anchors for this object and its copies; it is skipped"));
            return;
        }
        let mut changed = n.clone();
        let mut skipped = false;
        apply(&mut changed, &row.op, 0, &mut skipped);
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
                // A cut adds at most an anchor at each end and drops the rest of the subpath:
                // counted as what it was given, so it is never refused.
                GeometryOp::Trim(_) | GeometryOp::Tidy(_) => sp.anchors.len(),
            })
            .fold(0usize, usize::saturating_add),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().fold(0usize, |a, c| a.saturating_add(projected(c, op, depth + 1)))
        }
        _ => 0,
    }
}

/// `op` on every path in `n`; `skipped` is set when something it can't change is passed over.
fn apply(n: &mut Node, op: &GeometryOp, depth: usize, skipped: &mut bool) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    match &mut n.kind {
        NodeKind::Path { guide: true, .. } => {}
        NodeKind::Path { path, live, .. } => {
            if let Some(new) = shaped(path, op) {
                *path = new;
                // The copy is a plain path now: a live shape would redraw its old outline.
                *live = None;
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                apply(Arc::make_mut(c), op, depth + 1, skipped);
            }
        }
        _ => *skipped = true,
    }
}

/// `path` after `op`, `None` when nothing changes.
fn shaped(path: &PathData, op: &GeometryOp) -> Option<PathData> {
    let mut changed = false;
    let mut subpaths: Vec<SubPath> = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let new = match op {
            GeometryOp::Resample(p) => resample::resample(sp, p),
            GeometryOp::Tidy(p) => tidy::tidy(sp, p),
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
        .filter(|i| i.transform.is_finite())
        .map(|i| {
            let mut c = object.clone();
            if i.transform != vectorcraft_geom::Affine::IDENTITY {
                c.transform(i.transform, true);
            }
            shaper.copy(&mut c);
            c
        })
        .collect();
    (copies, shaper.notes)
}

/// `path` after `op` (as it is when `op` leaves it alone): bare geometry, without the caps.
pub fn shape_path(path: &PathData, op: &GeometryOp) -> PathData {
    shaped(path, op).unwrap_or_else(|| path.clone())
}
