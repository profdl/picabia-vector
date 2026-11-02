//! `transformEach`.
//!
//! Every instance gets `L = translate(offset) · rotate(rotate) · scale(sx, sy)` applied around its
//! own pivot, in its own frame (`S_i · pivot · L · pivot⁻¹`). With `progressive` the amounts are
//! scaled by `t = i / (n - 1)` (0 when n = 1): offset and rotation by `t`, scale from 1 to the
//! full factor (`1 + (s - 1) · t`), so the first instance is untouched and the last gets the full
//! amount. `jitter` multiplies each amount by `1 + jitter · r` with `r` in -1..=1 drawn per
//! instance and channel from `jitterSeed` and the instance index.
//!
//! A field with a per-copy expression (`rotateExpr`…, see [`crate::expr`]) takes the expression's
//! value for each instance as its full amount (no `progressive` ramp; jitter still applies),
//! clamped to the field's range; a result that isn't finite falls back to the field's value.

use vectorcraft_geom::{Affine, Vec2};

use super::rng::Stream;
use super::{Clamp, Ctx, Stage, StageResult};
use crate::bind::{PerCopy, PerCopyExpr};
use crate::expr::{Env, Expr, compile};
use crate::model::TransformEach;
use crate::{MAX_ANGLE, MAX_LENGTH, MAX_SCALE};

const MIN_SCALE: f64 = 1.0e-6;

/// What drives a channel per copy: its own per-copy expression, or a bound one
/// ([`crate::bind`], which counts the copies it gives no number for).
enum Driver {
    Own(Expr),
    Bound(PerCopyExpr),
}

/// One channel: its clamped value, its expression, and the range expression results are kept in.
struct Channel {
    value: f64,
    expr: Option<Driver>,
    lo: f64,
    hi: f64,
}

impl Channel {
    /// The amount for copy `i` of `n`: the expression's value, or `ramp(value)` without one.
    fn amount(&self, i: usize, n: usize, ramp: impl Fn(f64) -> f64) -> f64 {
        match &self.expr {
            Some(Driver::Own(e)) => e.eval_or(&Env::new(i, n, self.value), self.value).clamp(self.lo, self.hi),
            Some(Driver::Bound(e)) => e.amount(i, n, self.value, self.lo, self.hi),
            None => ramp(self.value),
        }
    }
}

pub(crate) fn stage(p: &TransformEach, _ctx: &Ctx, per_copy: &PerCopy) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let (ox, oy) = (c.length("offsetX", p.offset_x), c.length("offsetY", p.offset_y));
    let rotate = c.angle("rotate", p.rotate);
    let (sx, sy) = (c.scale("scaleX", p.scale_x), c.scale("scaleY", p.scale_y));
    let jitter = c.within("jitter", p.jitter, 0.0, 10.0, 0.0);
    let mut ch = |field: &str, value: f64, text: &Option<String>, lo: f64, hi: f64| Channel {
        value,
        expr: match per_copy.get(field) {
            Some(b) => Some(Driver::Bound(b.clone())),
            None => compile(text.as_deref(), &crate::expr_key(field), &mut warnings).map(Driver::Own),
        },
        lo,
        hi,
    };
    let chans = [
        ch("offsetX", ox, &p.offset_x_expr, -MAX_LENGTH, MAX_LENGTH),
        ch("offsetY", oy, &p.offset_y_expr, -MAX_LENGTH, MAX_LENGTH),
        ch("rotate", rotate, &p.rotate_expr, -MAX_ANGLE, MAX_ANGLE),
        ch("scaleX", sx, &p.scale_x_expr, 0.0, MAX_SCALE),
        ch("scaleY", sy, &p.scale_y_expr, 0.0, MAX_SCALE),
    ];
    let (progressive, seed) = (p.progressive, p.jitter_seed);
    let each = move |_: &crate::eval::Instance, i: usize, n: usize| {
        let t = if progressive && n > 1 {
            i as f64 / (n - 1) as f64
        } else if progressive {
            0.0
        } else {
            1.0
        };
        let mut j = [1.0; 5];
        if jitter > 0.0 {
            let mut rng = Stream::new(seed, i);
            for v in &mut j {
                *v = 1.0 + jitter * rng.signed();
            }
        }
        let [cx, cy, cr, csx, csy] = &chans;
        let lin = |v: f64| v * t;
        let off = Vec2::new(cx.amount(i, n, lin) * j[0], cy.amount(i, n, lin) * j[1]);
        let turn = (cr.amount(i, n, lin) * j[2]).rem_euclid(360.0).to_radians();
        // A scale's amount is its factor: the ramp goes from 1 to it, jitter varies its distance from 1.
        let factor = |ch: &Channel, jit: f64| {
            let f = ch.amount(i, n, |s| 1.0 + (s - 1.0) * t);
            (1.0 + (f - 1.0) * jit).clamp(MIN_SCALE, MAX_SCALE)
        };
        let (fx, fy) = (factor(csx, j[3]), factor(csy, j[4]));
        Affine::translate(off) * Affine::rotate(turn) * Affine::scale_non_uniform(fx, fy)
    };
    StageResult { stage: Stage::Each(Box::new(each)), guides: Vec::new(), warnings }
}
