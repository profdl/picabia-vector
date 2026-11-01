//! `linearArray`.
//!
//! Copy `k` (0-based; copy 0 is the original) is
//! `T_k = translate(k · offset) · rotate_about(k · rotateEach, pivot) · scale_about(scaleEach^k, pivot)`:
//! each copy turns and scales around its own centre, then moves `k` offsets along.

use vectorcraft_geom::{Affine, Vec2};

use super::{Clamp, Ctx, Stage, StageResult};
use crate::eval::Guide;
use crate::model::LinearArray;

/// Each copy's scale stays within this range (so a copy is never singular or astronomically big).
const MIN_COPY_SCALE: f64 = 1.0e-6;
const MAX_COPY_SCALE: f64 = 1.0e6;

pub(crate) fn stage(p: &LinearArray, ctx: &Ctx) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let count = c.count("count", p.count);
    let offset = Vec2::new(c.length("offsetX", p.offset_x), c.length("offsetY", p.offset_y));
    let rotate = c.angle("rotateEach", p.rotate_each);
    let scale = c.scale("scaleEach", p.scale_each);
    let mut copies = Vec::with_capacity(count);
    for k in 0..count {
        let kf = k as f64;
        // Reduced mod 360 so k · angle stays exact for big counts.
        let turn = (kf * rotate).rem_euclid(360.0).to_radians();
        let s = scale.powf(kf).clamp(MIN_COPY_SCALE, MAX_COPY_SCALE);
        let s = if k == 0 { 1.0 } else { s };
        copies.push(Affine::translate(offset * kf) * Affine::rotate_about(turn, ctx.pivot) * Affine::scale_about(s, ctx.pivot));
    }
    let last = ctx.pivot + offset * (count.saturating_sub(1) as f64);
    let guides = vec![Guide::Line { from: ctx.pivot, to: last }, Guide::Point(ctx.pivot), Guide::Point(last)];
    StageResult { stage: Stage::Array(copies), guides, warnings }
}
