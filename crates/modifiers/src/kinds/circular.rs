//! `circularArray`.
//!
//! Copy `k` sits at ring angle `a_k = startAngle + k · step` (degrees, clockwise on screen) where
//! `step = sweep / count` for a full turn (|sweep| >= 360: no duplicate at the end) and
//! `sweep / (count - 1)` otherwise. The original is copy 0 and lies on the ring at `startAngle`;
//! the ring centre is `radius` away from it (opposite the start direction), moved by
//! `centerX`/`centerY`. Copy `k` moves the layer's pivot to the ring point at `a_k` and, with
//! `align`, turns it by `a_k - a_0` about its own pivot so it keeps facing along the ring.
//! With a centre offset of 0 copy 0 is the identity.

use vectorcraft_geom::{Affine, Point, Vec2};

use super::{Clamp, Ctx, Stage, StageResult};
use crate::eval::Guide;
use crate::model::CircularArray;

pub(crate) fn stage(p: &CircularArray, ctx: &Ctx) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let count = c.count("count", p.count);
    let radius = c.size("radius", p.radius);
    let start = c.angle("startAngle", p.start_angle);
    let sweep = c.within("sweep", p.sweep, -360.0, 360.0, 360.0);
    let offset = Vec2::new(c.length("centerX", p.center_x), c.length("centerY", p.center_y));
    let step = if sweep.abs() >= 360.0 {
        sweep / count as f64
    } else if count > 1 {
        sweep / (count - 1) as f64
    } else {
        0.0
    };
    let dir = |deg: f64| {
        let r = deg.to_radians();
        Vec2::new(r.cos(), r.sin())
    };
    let centre: Point = ctx.pivot - dir(start) * radius + offset;
    let on_ring = |k: usize| centre + dir(start + step * k as f64) * radius;
    let first = on_ring(0);
    let mut copies = Vec::with_capacity(count);
    for k in 0..count {
        let turn = if p.align { (step * k as f64).rem_euclid(360.0).to_radians() } else { 0.0 };
        let to = on_ring(k) - first + offset;
        copies.push(Affine::translate(to) * Affine::rotate_about(turn, ctx.pivot));
    }
    let guides = vec![Guide::Circle { center: centre, radius }, Guide::Point(centre)];
    StageResult { stage: Stage::Array(copies), guides, warnings }
}
