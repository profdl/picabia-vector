//! Stages 6–7 together: find the islands of the cleaned cut and hold them, by ties, by the
//! pattern lattice (then ties for what it misses), or not at all (gauze); then look again so no
//! island is reported held that still floats.

use crate::ops::BoolOp;
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Vec2};
use vectorcraft_modifiers::{Stencil, StencilPatternWhere, StencilTieMode};

use crate::Island;
use crate::islands::Tree;
use crate::pattern::{self, Lattice};
use crate::rings;
use crate::ties::{self, Bar, TieParams};

/// The cut once the islands are held.
#[derive(Debug, Default)]
pub(crate) struct Held {
    pub cut: BezPath,
    /// Tie bars and lattice bars now in the paper.
    pub ties: BezPath,
    /// The lattice bars alone (`pattern` mode).
    pub lattice: BezPath,
    pub islands: Vec<Island>,
    pub ties_placed: usize,
    pub pattern_held: usize,
    pub untied: usize,
    /// The bounds of the islands still floating after the hold (`untied` of them).
    pub untied_bounds: Vec<Rect>,
    pub warnings: Vec<String>,
}

pub(crate) fn bars_path(bars: &[Bar]) -> BezPath {
    rings::to_path(&bars.iter().map(|b| b.to_vec()).collect::<Vec<_>>())
}

/// `cut` minus the tie `bars` (the bars become paper).
pub(crate) fn subtract_bars(cut: &BezPath, bars: &[Bar]) -> Result<BezPath, crate::ops::PathOpsError> {
    if bars.is_empty() {
        return Ok(cut.clone());
    }
    crate::ops::boolean_step(cut, &bars_path(bars), BoolOp::Subtract)
}

/// The lattice step: `(cut with the lattice in it, lattice now paper)`, or the cut unchanged with
/// a warning.
fn lattice(cut: &BezPath, tree: &Tree, sheet: Rect, p: &Stencil, mpu: f64, warnings: &mut Vec<String>) -> (BezPath, BezPath) {
    let l = Lattice { kind: p.pattern_type, width: p.pattern_width_mm / mpu, spacing: p.pattern_spacing_mm / mpu, angle: p.pattern_angle };
    if p.pattern_width_mm < p.min_bridge_mm {
        warnings.push(format!("the pattern bars ({} mm) are thinner than the minimum bridge ({} mm)", p.pattern_width_mm, p.min_bridge_mm));
    }
    let Some(bars) = pattern::bars(sheet, &l) else {
        warnings.push(format!("the pattern is too fine for the sheet (over {} bars); it is left out", pattern::MAX_BARS));
        return (cut.clone(), BezPath::new());
    };
    match pattern::apply(cut, tree, &bars, p.pattern_where == StencilPatternWhere::Islands) {
        Ok(r) => r,
        Err(_) => {
            warnings.push("couldn't lay the pattern into the cut; it is left out".into());
            (cut.clone(), BezPath::new())
        }
    }
}

pub(crate) fn tie_params(p: &Stencil, mpu: f64, sheet: Rect) -> TieParams {
    TieParams {
        sheet,
        width: p.tie_width_mm / mpu,
        per_island: p.ties_per_island,
        spread: p.tie_spread,
        dir: p.tie_angle.map(|a| {
            let (c, s) = pattern::cos_sin(a);
            Vec2::new(c, -s)
        }),
        seed: p.tie_seed,
    }
}

/// Find and hold the islands of `cut` (plain contours inside `sheet`) for the clamped params `p`
/// at `mpu` mm per unit.
pub(crate) fn hold(cut: BezPath, sheet: Rect, p: &Stencil, mpu: f64) -> Held {
    let mut held = Held::default();
    let Some(mut tree0) = Tree::new(&cut) else {
        held.warnings.push("the cut is too detailed to look for islands".into());
        held.cut = cut;
        return held;
    };
    let originals = tree0.islands();
    if originals.is_empty() {
        held.cut = cut;
        return held;
    }
    held.islands = originals
        .iter()
        .map(|&i| Island {
            bounds: tree0.bounds.get(i).copied().unwrap_or(Rect::ZERO),
            tied: 0,
            pattern_held: false,
            nested_depth: tree0.nested_depth(i),
        })
        .collect();
    if p.tie_mode == StencilTieMode::Gauze {
        held.untied = originals.len();
        held.untied_bounds = held.islands.iter().map(|i| i.bounds).collect();
        held.warnings.push(format!("{} islands are left untied (gauze mode): a mesh must hold them", originals.len()));
        held.cut = cut;
        return held;
    }
    let probes: Vec<Option<Point>> = originals.iter().map(|&i| tree0.inside_point(i)).collect();
    // Pattern: the lattice first, then ties for the islands still floating.
    let (cut1, mut ties_path, mut tree1) = if p.tie_mode == StencilTieMode::Pattern {
        let (cut1, in_paper) = lattice(&cut, &tree0, sheet, p, mpu, &mut held.warnings);
        match Tree::new(&cut1) {
            Some(t) => (cut1, in_paper, t),
            None => (cut.clone(), BezPath::new(), tree0),
        }
    } else {
        (cut.clone(), BezPath::new(), tree0)
    };
    let lattice_len = ties_path.elements().len();
    let floating = tree1.islands();
    let tp = tie_params(p, mpu, sheet);
    let (bars, counts) = ties::tie(&mut tree1, &floating, &tp);
    if !bars.is_empty() && p.tie_width_mm < p.min_bridge_mm {
        held.warnings.push(format!("the ties ({} mm) are thinner than the minimum bridge ({} mm)", p.tie_width_mm, p.min_bridge_mm));
    }
    let cut2 = match subtract_bars(&cut1, &bars) {
        Ok(c) => {
            held.ties_placed = bars.len();
            ties_path.extend(bars_path(&bars).iter());
            c
        }
        Err(_) => {
            held.warnings.push("couldn't merge the ties into the paper; the islands are left untied".into());
            cut1
        }
    };
    let placed = held.ties_placed > 0;
    for (k, isl) in held.islands.iter_mut().enumerate() {
        let probe = probes.get(k).copied().flatten();
        let deepest = probe.map(|pt| tree1.deepest_containing(pt));
        let floating_ring = match deepest {
            // In the sheet's paper now: the lattice holds it.
            Some(None) => None,
            Some(Some(r)) if tree1.depth.get(r).is_some_and(|d| d % 2 == 1) => Some(r),
            Some(Some(_)) => None,
            None => originals.get(k).copied(),
        };
        match floating_ring {
            None if p.tie_mode == StencilTieMode::Pattern && probe.is_some() => isl.pattern_held = true,
            None => {}
            Some(r) => {
                let at = floating.iter().position(|&f| f == r);
                isl.tied = if placed { at.and_then(|a| counts.get(a).copied()).unwrap_or(0) } else { 0 };
            }
        }
    }
    held.pattern_held = held.islands.iter().filter(|i| i.pattern_held).count();
    // Look again: whatever still floats is untied.
    held.untied_bounds = match Tree::new(&cut2) {
        Some(t) => t.islands().iter().map(|&i| t.bounds.get(i).copied().unwrap_or(Rect::ZERO)).collect(),
        None => held.islands.iter().filter(|i| i.tied == 0 && !i.pattern_held).map(|i| i.bounds).collect(),
    };
    held.untied = held.untied_bounds.len();
    if held.untied > 0 {
        held.warnings.push(format!("{} islands couldn't be tied and would fall out", held.untied));
    }
    held.cut = cut2;
    if p.tie_mode == StencilTieMode::Pattern {
        held.lattice = ties_path.clone();
        held.lattice.truncate(lattice_len);
    }
    held.ties = ties_path;
    held
}
