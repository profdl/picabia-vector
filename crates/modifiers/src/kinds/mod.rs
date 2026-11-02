//! One module per modifier kind. Each turns its params into a [`Stage`]; [`crate::evaluate`] does
//! the composition, caps and indexing. Adding a kind = a module here plus one arm in [`stage`].
//!
//! Unknown kinds ([`ModifierKind::Other`]) evaluate as [`Stage::Unchanged`].

mod circular;
mod copy_to_shape;
mod grid;
mod linear;
mod mirror;
mod randomize;
mod rng;
mod stencil;
mod transform_each;
mod wallpaper;

use vectorcraft_geom::{Affine, Point, Rect};

use crate::eval::{Guide, Instance};
use crate::model::ModifierKind;
use crate::target::TargetResolver;
use crate::{MAX_ANGLE, MAX_COUNT, MAX_LENGTH, MAX_SCALE};

/// What a stage sees: the pivot (the layer's bounds centre, parent space), the bounds, and how
/// many instances the stages before it produced.
pub(crate) struct Ctx {
    pub pivot: Point,
    pub bounds: Rect,
    pub n_prev: usize,
}

/// A local transform per instance: `(instance, index, count) -> L_i`, applied around the pivot.
pub(crate) type EachFn = Box<dyn Fn(&Instance, usize, usize) -> Affine>;

/// What one modifier does to the set of instances.
pub(crate) enum Stage {
    /// Pass the set through.
    Unchanged,
    /// Repeat the whole set once per copy transform (`T_0` is usually the identity).
    Array(Vec<Affine>),
    /// Transform each instance around the pivot: `S_i * (pivot · L_i · pivot⁻¹)`.
    Each(EachFn),
    /// The stage would produce this many copies of the set, more than the cap allows; nothing was
    /// allocated.
    TooMany(usize),
}

pub(crate) struct StageResult {
    pub stage: Stage,
    pub guides: Vec<Guide>,
    pub warnings: Vec<String>,
}

impl StageResult {
    pub fn unchanged() -> Self {
        StageResult { stage: Stage::Unchanged, guides: Vec::new(), warnings: Vec::new() }
    }
}

/// The stage modifier `kind` makes in `ctx` (`targets` finds copy-to-shape targets; `per_copy`
/// holds the row's bound per-copy expressions, [`crate::bind`]).
pub(crate) fn stage(kind: &ModifierKind, ctx: &Ctx, targets: &dyn TargetResolver, per_copy: &crate::bind::PerCopy) -> StageResult {
    match kind {
        ModifierKind::LinearArray(p) => linear::stage(p, ctx),
        ModifierKind::CircularArray(p) => circular::stage(p, ctx),
        ModifierKind::GridArray(p) => grid::stage(p, ctx),
        ModifierKind::Mirror(p) => mirror::stage(p, ctx),
        ModifierKind::TransformEach(p) => transform_each::stage(p, ctx, per_copy),
        ModifierKind::Randomize(p) => randomize::stage(p, ctx, per_copy),
        ModifierKind::Wallpaper(p) => wallpaper::stage(p, ctx),
        ModifierKind::CopyToShape(p) => copy_to_shape::stage(p, ctx, targets),
        // Terminal: the evaluator stops before it; were it reached, it would add nothing.
        ModifierKind::Stencil(_) => StageResult::unchanged(),
        // Geometry rows place no copies: the evaluator collects them (`crate::shape`).
        ModifierKind::Resample(_) | ModifierKind::Trim(_) | ModifierKind::Tidy(_) => StageResult::unchanged(),
        ModifierKind::Deform(_) => StageResult::unchanged(),
        ModifierKind::Displace(_) | ModifierKind::Screen(_) => StageResult::unchanged(),
        ModifierKind::Other { .. } => StageResult::unchanged(),
    }
}

/// The translation lattice of a `wallpaper` or `gridArray` row in `ctx` ([`crate::Output::repeat`]);
/// `None` for every other kind (and a 1 × 1 grid).
pub(crate) fn repeat(kind: &ModifierKind, ctx: &Ctx) -> Option<crate::eval::Repeat> {
    match kind {
        ModifierKind::Wallpaper(p) => Some(wallpaper::repeat(p, ctx)),
        ModifierKind::GridArray(p) => grid::repeat(p, ctx),
        _ => None,
    }
}

/// [`crate::model::Stencil::clamped`].
pub(crate) fn stencil_clamped(p: &crate::model::Stencil) -> (crate::model::Stencil, Vec<String>) {
    stencil::clamped(p)
}

/// [`crate::Modifier::set_props`]'s material rule for a stencil (see `stencil::apply_material`).
pub(crate) fn stencil_apply_material(
    old: &crate::model::Stencil,
    new: &mut crate::model::Stencil,
    props: &serde_json::Map<String, serde_json::Value>,
) {
    stencil::apply_material(old, new, props);
}

/// Clamp helpers: every param reaches a transform through one of these.
pub(crate) struct Clamp<'a> {
    pub warnings: &'a mut Vec<String>,
}

impl Clamp<'_> {
    fn bounded(&mut self, name: &str, v: f64, lo: f64, hi: f64, default: f64) -> f64 {
        if !v.is_finite() {
            self.warnings.push(format!("`{name}` isn't a finite number; using {default}"));
            return default;
        }
        if v < lo || v > hi {
            self.warnings.push(format!("`{name}` is limited to {lo}..{hi}"));
        }
        v.clamp(lo, hi)
    }

    /// A length in points, within ±[`MAX_LENGTH`].
    pub fn length(&mut self, name: &str, v: f64) -> f64 {
        self.bounded(name, v, -MAX_LENGTH, MAX_LENGTH, 0.0)
    }

    /// An angle in degrees, within ±[`MAX_ANGLE`].
    pub fn angle(&mut self, name: &str, v: f64) -> f64 {
        self.bounded(name, v, -MAX_ANGLE, MAX_ANGLE, 0.0)
    }

    /// A scale factor within `0..=MAX_SCALE`.
    pub fn scale(&mut self, name: &str, v: f64) -> f64 {
        self.bounded(name, v, 0.0, MAX_SCALE, 1.0)
    }

    /// A copy count within `1..=MAX_COUNT` (0 copies would hide the layer; it reads as 1).
    pub fn count(&mut self, name: &str, v: u32) -> usize {
        if v > MAX_COUNT {
            self.warnings.push(format!("`{name}` is limited to {MAX_COUNT}"));
        }
        usize::try_from(v.clamp(1, MAX_COUNT)).unwrap_or(1)
    }
}

impl Clamp<'_> {
    /// A non-negative length within `0..=MAX_LENGTH`.
    pub fn size(&mut self, name: &str, v: f64) -> f64 {
        self.bounded(name, v, 0.0, MAX_LENGTH, 0.0)
    }

    /// A value within `lo..=hi`, `default` when not finite.
    pub fn within(&mut self, name: &str, v: f64, lo: f64, hi: f64, default: f64) -> f64 {
        self.bounded(name, v, lo, hi, default)
    }
}

/// A wallpaper group's lattice basis `(a, b)` for a `w × h` cell and oblique angle `angle`
/// (degrees): what the canvas handles place the cell's corners by ([`crate::handles`]).
pub(crate) fn wallpaper_lattice(g: crate::model::WallpaperGroup, w: f64, h: f64, angle: f64) -> ([f64; 2], [f64; 2]) {
    wallpaper::lattice(g, w, h, angle)
}

/// The crate's deterministic stream for `(seed, index)` (the geometry rows' random sort).
pub(crate) fn rng_stream(seed: u32, index: usize) -> rng::Stream {
    rng::Stream::new(seed, index)
}

#[cfg(test)]
pub(crate) fn rng_for_tests(seed: u32, index: usize) -> rng::Stream {
    rng::Stream::new(seed, index)
}
