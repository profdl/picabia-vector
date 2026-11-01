//! `gridArray`.
//!
//! `rows × cols` copies in row-major order (copy `k` = row `k / cols`, column `k % cols`; copy 0 is
//! the original at the top left). `gapX`/`gapY` are the space *between neighbouring copies'
//! bounds*, so the step is `bounds width + gapX` by `bounds height + gapY`. A negative gap
//! overlaps copies.

use vectorcraft_geom::{Affine, Point, Vec2};

use super::{Clamp, Ctx, Stage, StageResult};
use crate::MAX_INSTANCES_PER_NODE;
use crate::eval::{Guide, Repeat};
use crate::model::GridArray;

/// The clamped `(rows, cols, step)` of a grid row: what [`stage`] and [`repeat`] both use.
fn layout(p: &GridArray, ctx: &Ctx, c: &mut Clamp<'_>) -> (usize, usize, Vec2) {
    let rows = c.count("rows", p.rows);
    let cols = c.count("cols", p.cols);
    let gap = Vec2::new(c.length("gapX", p.gap_x), c.length("gapY", p.gap_y));
    (rows, cols, Vec2::new(ctx.bounds.width() + gap.x, ctx.bounds.height() + gap.y))
}

/// The row's translation lattice: `a = (step.x, 0)`, `b = (0, step.y)` at the layer bounds'
/// top-left; a single column has `a` = 0 (no repeat along it), a single row `b` = 0. `None` for a
/// 1 × 1 grid (no repeat at all).
pub(crate) fn repeat(p: &GridArray, ctx: &Ctx) -> Option<Repeat> {
    let (rows, cols, step) = layout(p, ctx, &mut Clamp { warnings: &mut Vec::new() });
    let a = if cols > 1 { Vec2::new(step.x, 0.0) } else { Vec2::ZERO };
    let b = if rows > 1 { Vec2::new(0.0, step.y) } else { Vec2::ZERO };
    (a != Vec2::ZERO || b != Vec2::ZERO).then_some(Repeat { origin: Point::new(ctx.bounds.x0, ctx.bounds.y0), a, b })
}

pub(crate) fn stage(p: &GridArray, ctx: &Ctx) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let (rows, cols, step) = layout(p, ctx, &mut c);
    let total = rows * cols;
    // Checked before allocating anything per copy.
    if total.saturating_mul(ctx.n_prev) > MAX_INSTANCES_PER_NODE {
        return StageResult { stage: Stage::TooMany(total), guides: Vec::new(), warnings };
    }
    let copies = (0..total).map(|k| Affine::translate(Vec2::new(step.x * (k % cols) as f64, step.y * (k / cols) as f64))).collect();
    // The lattice extents: the outline of the whole grid.
    let (x0, y0) = (ctx.bounds.x0, ctx.bounds.y0);
    let x1 = ctx.bounds.x1 + step.x * (cols - 1) as f64;
    let y1 = ctx.bounds.y1 + step.y * (rows - 1) as f64;
    let (a, b, d, e) = (Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1));
    let guides = vec![Guide::Line { from: a, to: b }, Guide::Line { from: b, to: d }, Guide::Line { from: d, to: e }, Guide::Line { from: e, to: a }];
    StageResult { stage: Stage::Array(copies), guides, warnings }
}
