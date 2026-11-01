//! `randomize`.
//!
//! Per instance `i`, four numbers `r` in -1..=1 come from the stream fixed by `(seed, i)`
//! (SplitMix64, see `rng`): shift `(position · r0, position · r1)` points, turn `rotation · r2`
//! degrees, scale `1 + scale · r3`, applied around the instance's own pivot in its own frame.
//! An instance's numbers depend only on `seed` and its index.
//!
//! An amount with a per-copy expression (`positionExpr`…, see [`crate::expr`]) is the
//! expression's value for that instance (`value` = the field), clamped to the field's range.

use vectorcraft_geom::{Affine, Vec2};

use super::rng::Stream;
use super::{Clamp, Ctx, Stage, StageResult};
use crate::bind::PerCopy;
use crate::expr::{Env, compile};
use crate::model::Randomize;
use crate::{MAX_ANGLE, MAX_LENGTH, MAX_SCALE};

const MIN_SCALE: f64 = 1.0e-6;

pub(crate) fn stage(p: &Randomize, _ctx: &Ctx, per_copy: &PerCopy) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let position = c.size("position", p.position);
    let rotation = c.within("rotation", p.rotation, 0.0, MAX_ANGLE, 0.0);
    let scale = c.within("scale", p.scale, 0.0, MAX_SCALE, 0.0);
    // A bound per-copy expression ([`crate::bind`]) drives its amount in place of its own.
    let mut driver = |field: &str, text: Option<&str>| match per_copy.get(field) {
        Some(b) => Some(Err(b.clone())),
        None => compile(text, &crate::expr_key(field), &mut warnings).map(Ok),
    };
    let exprs = [
        (position, driver("position", p.position_expr.as_deref()), MAX_LENGTH),
        (rotation, driver("rotation", p.rotation_expr.as_deref()), MAX_ANGLE),
        (scale, driver("scale", p.scale_expr.as_deref()), MAX_SCALE),
    ];
    let seed = p.seed;
    let each = move |_: &crate::eval::Instance, i: usize, n: usize| {
        let [position, rotation, scale] = exprs.each_ref().map(|(value, e, hi)| match e {
            Some(Ok(e)) => e.eval_or(&Env::new(i, n, *value), *value).clamp(0.0, *hi),
            Some(Err(b)) => b.amount(i, n, *value, 0.0, *hi),
            None => *value,
        });
        let mut rng = Stream::new(seed, i);
        let (r0, r1, r2, r3) = (rng.signed(), rng.signed(), rng.signed(), rng.signed());
        let s = (1.0 + scale * r3).clamp(MIN_SCALE, MAX_SCALE);
        Affine::translate(Vec2::new(position * r0, position * r1)) * Affine::rotate((rotation * r2).to_radians()) * Affine::scale(s)
    };
    StageResult { stage: Stage::Each(Box::new(each)), guides: Vec::new(), warnings }
}
