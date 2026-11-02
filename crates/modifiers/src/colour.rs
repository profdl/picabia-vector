//! The `color` row's per-copy colour shifts (`docs/picabia/colour.md`).
//!
//! A per-instance step: it runs on the final copies, after every array row, wherever it sits, and
//! adds no copies and moves nothing. For copy `k` of `n` (counted in the row's `order`):
//!
//! 1. the ramp multiplier `m` is 1 with `progressive` off; on, `u = k / (n - 1)` (`1 - u` with
//!    `reverse`) and `m = u × amount`, and with `n = 1`, `m = amount`;
//! 2. jitter per channel is `r × jitter`, `r` in -1..1 fixed by the seed, the copy's index and the
//!    channel alone ([`crate::kinds`]' SplitMix64);
//! 3. each channel's shift is `field × m + jitter` (a per-copy expression replaces `field × m`).
//!
//! The crate stays pure: it gives the numbers ([`ColourShift`] per copy and row,
//! [`crate::Output::colours`]); whoever builds the copies recolours them
//! ([`vectorcraft_color::oklch_shift`]).

use vectorcraft_color::oklch_shift::OklchShift;
use vectorcraft_geom::Point;

use crate::bind::{PerCopy, PerCopyExpr};
use crate::eval::{ArrayStage, Instance};
use crate::expr::{Env, Expr, compile};
use crate::kinds::Clamp;
use crate::model::{Colour, ColourOrder, ColourStroke};

/// What one `color` row does to one copy: the shift of its fills and, unless strokes keep their
/// colour, of its strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColourShift {
    pub fill: OklchShift,
    /// `None`: strokes keep their colour (`stroke: none`).
    pub stroke: Option<OklchShift>,
}

impl ColourShift {
    /// Whether it changes nothing.
    pub fn is_zero(&self) -> bool {
        self.fill.is_zero() && self.stroke.is_none_or(|s| s.is_zero())
    }
}

/// What drives a fill channel per copy: its own expression or a bound one.
enum Driver {
    Own(Expr),
    Bound(PerCopyExpr),
}

/// One `color` row, clamped and ready to run on the final copies.
pub(crate) struct Step {
    p: Colour,
    drivers: [Option<Driver>; 3],
}

const SHIFT: f64 = 100.0;

/// Row `p`, clamped (with warnings), its fill expressions compiled (bound per-copy ones from
/// `per_copy` take their place).
pub(crate) fn step(p: &Colour, per_copy: &PerCopy, warnings: &mut Vec<String>) -> Step {
    let mut q = p.clone();
    {
        let mut c = Clamp { warnings: &mut *warnings };
        let mut sh = |name: &str, v: f64| c.within(name, v, -SHIFT, SHIFT, 0.0);
        q.hue = sh("hue", p.hue);
        q.chroma = sh("chroma", p.chroma);
        q.lightness = sh("lightness", p.lightness);
        q.stroke_hue = sh("strokeHue", p.stroke_hue);
        q.stroke_chroma = sh("strokeChroma", p.stroke_chroma);
        q.stroke_lightness = sh("strokeLightness", p.stroke_lightness);
    }
    let mut c = Clamp { warnings: &mut *warnings };
    q.amount = c.within("amount", p.amount, 0.0, 1.0, 1.0);
    let mut jit = |name: &str, v: f64| c.within(name, v, 0.0, SHIFT, 0.0);
    q.jitter_hue = jit("jitterHue", p.jitter_hue);
    q.jitter_chroma = jit("jitterChroma", p.jitter_chroma);
    q.jitter_lightness = jit("jitterLightness", p.jitter_lightness);
    q.stroke_jitter_hue = jit("strokeJitterHue", p.stroke_jitter_hue);
    q.stroke_jitter_chroma = jit("strokeJitterChroma", p.stroke_jitter_chroma);
    q.stroke_jitter_lightness = jit("strokeJitterLightness", p.stroke_jitter_lightness);
    q.array = p.array.max(1);
    let mut driver = |field: &str, text: Option<&str>| match per_copy.get(field) {
        Some(b) => Some(Driver::Bound(b.clone())),
        None => compile(text, &crate::expr_key(field), warnings).map(Driver::Own),
    };
    let drivers =
        [driver("hue", p.hue_expr.as_deref()), driver("chroma", p.chroma_expr.as_deref()), driver("lightness", p.lightness_expr.as_deref())];
    Step { p: q, drivers }
}

/// Where each copy stands in `order`: `(k, n, jitter index)` per copy.
fn positions(p: &Colour, set: &[Instance], pivot: Point, stages: &[ArrayStage]) -> Vec<(usize, usize, usize)> {
    let n = set.len();
    let key = |inst: &Instance| {
        let c = inst.transform * pivot;
        match p.order {
            ColourOrder::LeftRight => c.x,
            ColourOrder::TopBottom => c.y,
            _ => c.x + c.y,
        }
    };
    match p.order {
        ColourOrder::Copies => (0..n).map(|i| (i, n, i)).collect(),
        ColourOrder::Array => {
            // Array rows counted from the top; past the last, the last; none: as `copies`.
            let Some(last) = stages.len().checked_sub(1) else {
                return (0..n).map(|i| (i, n, i)).collect();
            };
            let a = (p.array.max(1) as usize - 1).min(last);
            let count = stages.get(a).map_or(1, |s| s.copies.max(1) as usize);
            set.iter()
                .enumerate()
                .map(|(i, inst)| {
                    let k = inst.stage_indices.get(a).map_or(0, |k| *k as usize).min(count - 1);
                    (k, count, i)
                })
                .collect()
        }
        ColourOrder::LeftRight | ColourOrder::TopBottom | ColourOrder::Diagonal => {
            let keys: Vec<f64> = set.iter().map(key).map(|v| if v.is_finite() { v } else { 0.0 }).collect();
            let mut order: Vec<usize> = (0..n).collect();
            // Stable: ties keep copy order.
            order.sort_by(|a, b| keys.get(*a).unwrap_or(&0.0).total_cmp(keys.get(*b).unwrap_or(&0.0)));
            let mut out = vec![(0, n, 0); n];
            for (rank, i) in order.into_iter().enumerate() {
                if let Some(slot) = out.get_mut(i) {
                    *slot = (rank, n, rank);
                }
            }
            out
        }
    }
}

impl Step {
    /// The ramp multiplier for copy `k` of `n`.
    fn ramp(&self, k: usize, n: usize) -> f64 {
        let p = &self.p;
        if !p.progressive {
            return 1.0;
        }
        if n <= 1 {
            return p.amount;
        }
        let u = k as f64 / (n - 1) as f64;
        let u = if p.reverse { 1.0 - u } else { u };
        u * p.amount
    }

    fn shift(&self, k: usize, n: usize, j: usize) -> ColourShift {
        let p = &self.p;
        let m = self.ramp(k, n);
        let jitter = |channel: u64, amount: f64| if amount > 0.0 { crate::kinds::rng_unit(p.seed, j, channel) * amount } else { 0.0 };
        let fields = [p.hue, p.chroma, p.lightness];
        let jitters = [p.jitter_hue, p.jitter_chroma, p.jitter_lightness];
        let mut fill = [0.0; 3];
        for (ch, slot) in fill.iter_mut().enumerate() {
            let value = fields.get(ch).copied().unwrap_or(0.0);
            let base = match self.drivers.get(ch).and_then(Option::as_ref) {
                Some(Driver::Own(e)) => e.eval_or(&Env::new(k, n, value), value * m).clamp(-SHIFT, SHIFT),
                Some(Driver::Bound(b)) => b.amount(k, n, value, -SHIFT, SHIFT),
                None => value * m,
            };
            *slot = base + jitter(ch as u64, jitters.get(ch).copied().unwrap_or(0.0));
        }
        let fill = OklchShift::new(fill[0], fill[1], fill[2]);
        let stroke = match p.stroke {
            ColourStroke::Same => Some(fill),
            ColourStroke::None => None,
            ColourStroke::Own => Some(OklchShift::new(
                p.stroke_hue * m + jitter(3, p.stroke_jitter_hue),
                p.stroke_chroma * m + jitter(4, p.stroke_jitter_chroma),
                p.stroke_lightness * m + jitter(5, p.stroke_jitter_lightness),
            )),
        };
        ColourShift { fill, stroke }
    }
}

/// The shifts of `steps` (in stack order) for every copy of `set`: one list per copy, one shift
/// per row; none without steps.
pub(crate) fn shifts(steps: &[Step], set: &[Instance], pivot: Point, stages: &[ArrayStage]) -> Vec<Vec<ColourShift>> {
    if steps.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Vec<ColourShift>> = (0..set.len()).map(|_| Vec::with_capacity(steps.len())).collect();
    for s in steps {
        for (slot, (k, n, j)) in out.iter_mut().zip(positions(&s.p, set, pivot, stages)) {
            slot.push(s.shift(k, n, j));
        }
    }
    out
}
