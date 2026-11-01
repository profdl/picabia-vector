//! `mirror`.
//!
//! Reflects the previous set across a line through the pivot, moved `offset` points
//! perpendicular to the line (vertical axis: along x; horizontal: along y; both: each line moves
//! `offset` along its own normal). `keepOriginal` on: the previous set plus its reflection(s)
//! (vertical/horizontal: 2 blocks; both: 4 blocks, order original, flipped left-right, flipped
//! top-bottom, flipped both). Off: only the reflection(s) (1 block, or the 3 reflected blocks for
//! `both`).

use vectorcraft_geom::{Affine, Point};

use super::{Clamp, Ctx, Stage, StageResult};
use crate::eval::Guide;
use crate::model::{Mirror, MirrorAxis};

pub(crate) fn stage(p: &Mirror, ctx: &Ctx) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let offset = c.length("offset", p.offset);
    let ax = ctx.pivot.x + offset;
    let ay = ctx.pivot.y + offset;
    // x' = 2·ax - x, y' = 2·ay - y.
    let flip_x = Affine::new([-1.0, 0.0, 0.0, 1.0, 2.0 * ax, 0.0]);
    let flip_y = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * ay]);
    let mut copies = Vec::new();
    match p.axis {
        MirrorAxis::Vertical => copies.push(flip_x),
        MirrorAxis::Horizontal => copies.push(flip_y),
        MirrorAxis::Both => copies.extend([flip_x, flip_y, flip_y * flip_x]),
    }
    if p.keep_original {
        copies.insert(0, Affine::IDENTITY);
    }
    // Axis lines reach past the layer so they are easy to see.
    let reach = (ctx.bounds.width().max(ctx.bounds.height()) * 1.5).max(100.0);
    let vertical = Guide::Line { from: Point::new(ax, ctx.pivot.y - reach), to: Point::new(ax, ctx.pivot.y + reach) };
    let horizontal = Guide::Line { from: Point::new(ctx.pivot.x - reach, ay), to: Point::new(ctx.pivot.x + reach, ay) };
    let guides = match p.axis {
        MirrorAxis::Vertical => vec![vertical],
        MirrorAxis::Horizontal => vec![horizontal],
        MirrorAxis::Both => vec![vertical, horizontal],
    };
    StageResult { stage: Stage::Array(copies), guides, warnings }
}
