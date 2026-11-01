//! `pattern` tie mode: a lattice of thin paper bars across the cut. Bars are `patternWidthMm` wide,
//! `patternSpacingMm` apart centre to centre, rotated by `patternAngle` and anchored at the sheet's
//! top-left corner (a bar's centre line passes through it), so they stay put when the design
//! changes. Only the cut changes: `cut −= lattice` (everywhere, or only in the cut-outs that
//! border an island), and the lattice inside the cut becomes paper.

use crate::ops::BoolOp;
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Vec2};
use vectorcraft_modifiers::StencilPatternType;

use crate::islands::Tree;
use crate::rings;

/// The most lattice bars: past it the lattice is left out (with a warning).
pub(crate) const MAX_BARS: usize = 4000;

/// `(cos, sin)` of `deg`, exact at multiples of 90.
pub(crate) fn cos_sin(deg: f64) -> (f64, f64) {
    if deg == 0.0 {
        (1.0, 0.0)
    } else if deg == 90.0 {
        (0.0, 1.0)
    } else if deg == 180.0 {
        (-1.0, 0.0)
    } else if deg == 270.0 {
        (0.0, -1.0)
    } else {
        let r = deg.to_radians();
        (r.cos(), r.sin())
    }
}

/// Lattice settings in layer units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lattice {
    pub kind: StencilPatternType,
    pub width: f64,
    pub spacing: f64,
    /// Degrees, counter-clockwise on the canvas (y down).
    pub angle: f64,
}

/// The lattice's bars covering `sheet` (positive rectangles, non-zero union), or `None` when it
/// would have more than [`MAX_BARS`].
pub(crate) fn bars(sheet: Rect, l: &Lattice) -> Option<BezPath> {
    let (c, s) = cos_sin(l.angle);
    // Along the "horizontal" bars, and across them (y down: counter-clockwise on screen).
    let u = Vec2::new(c, -s);
    let n = Vec2::new(s, c);
    let o = Point::new(sheet.x0, sheet.y0);
    let corners = [o, Point::new(sheet.x1, sheet.y0), Point::new(sheet.x1, sheet.y1), Point::new(sheet.x0, sheet.y1)];
    let range = |axis: Vec2| corners.iter().map(|&p| (p - o).dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    let families: &[(Vec2, Vec2)] = match l.kind {
        StencilPatternType::Grid => &[(u, n), (n, u)],
        StencilPatternType::Horizontal => &[(u, n)],
        StencilPatternType::Vertical => &[(n, u)],
    };
    let mut count = 0usize;
    let mut plans = Vec::new();
    for &(along, across) in families {
        let (a0, a1) = range(along);
        let (c0, c1) = range(across);
        let k0 = ((c0 - l.width) / l.spacing).floor();
        let k1 = ((c1 + l.width) / l.spacing).ceil();
        if !(k0.is_finite() && k1.is_finite()) || k1 - k0 > MAX_BARS as f64 {
            return None;
        }
        count += (k1 - k0) as usize + 1;
        plans.push((along, across, a0 - l.width, a1 + l.width, k0 as i64, k1 as i64));
    }
    if count > MAX_BARS {
        return None;
    }
    let mut out = BezPath::new();
    let hw = l.width / 2.0;
    for (along, across, a0, a1, k0, k1) in plans {
        for k in k0..=k1 {
            let mid = k as f64 * l.spacing;
            let quad = [
                o + along * a0 + across * (mid - hw),
                o + along * a1 + across * (mid - hw),
                o + along * a1 + across * (mid + hw),
                o + along * a0 + across * (mid + hw),
            ];
            let quad = if crate::islands::ring_area(&quad) < 0.0 { [quad[3], quad[2], quad[1], quad[0]] } else { quad };
            out.extend(rings::to_path(&[quad.to_vec()]).iter());
        }
    }
    Some(out)
}

/// `cut` with the lattice `bars` removed (from every cut-out, or with `islands_only` from those
/// bordering an island: cut outlines with a hole), and the lattice that became paper.
pub(crate) fn apply(cut: &BezPath, tree: &Tree, bars: &BezPath, islands_only: bool) -> Result<(BezPath, BezPath), crate::ops::PathOpsError> {
    let (chosen, rest) = if islands_only {
        let n = tree.rings.len();
        let mut has_hole = vec![false; n];
        for i in 0..n {
            if tree.depth.get(i).is_some_and(|d| d % 2 == 1)
                && let Some(slot) = tree.parent.get(i).copied().flatten().and_then(|p| has_hole.get_mut(p))
            {
                *slot = true;
            }
        }
        // A contour belongs to the cut-out it outlines (even depth) or bounds (a hole: its parent's).
        let owner = |i: usize| if tree.depth.get(i).is_some_and(|d| d % 2 == 1) { tree.parent.get(i).copied().flatten() } else { Some(i) };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (i, r) in tree.rings.iter().enumerate() {
            if owner(i).is_some_and(|o| has_hole.get(o).copied().unwrap_or(false)) { a.push(r.clone()) } else { b.push(r.clone()) }
        }
        (rings::to_path(&a), rings::to_path(&b))
    } else {
        (cut.clone(), BezPath::new())
    };
    if chosen.elements().is_empty() {
        return Ok((cut.clone(), BezPath::new()));
    }
    let mut out = crate::ops::boolean_step(&chosen, bars, BoolOp::Subtract)?;
    let paper = crate::ops::boolean_step(&chosen, bars, BoolOp::Intersect)?;
    out.extend(rest.iter());
    Ok((out, paper))
}
