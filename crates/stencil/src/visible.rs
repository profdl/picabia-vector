//! Stage 1: what each colour shows once everything above it is painted (visible regions by paint
//! order).
//!
//! Every paint is flattened, simplified to the tolerance and normalised (one sweep each, non-zero)
//! into plain contours: outers wound positively, holes negatively, so concatenating normalised
//! paths *is* their union under the non-zero rule, with no sweep. Paints are then grouped into runs
//! of consecutive same-colour paints (merged before anything is subtracted) and walked from the top
//! run down: `visible = run − covered`, `covered ∪= run`, where `covered` is just the concatenation
//! of the normalised paints above whose bounds meet the run's (no union sweep, and far-away paints
//! cost nothing). Each run costs one sweep. No arrangement (`faces()`) is built.

use crate::ops::BoolOp;
use vectorcraft_geom::kurbo::{BezPath, Rect, Shape};

use crate::{MAX_POINTS, Paint, rings};

/// One colour's visible region.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// The colour, 8-bit sRGB (rounded from the paint's).
    pub colour: [u8; 3],
    /// What shows of it, plain contours (even-odd and non-zero agree), layer units. Empty when the
    /// colour is fully covered.
    pub path: BezPath,
    /// The visible area, layer units².
    pub area: f64,
}

/// The output of [`visible_regions`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visible {
    /// One region per distinct colour, in order of first appearance from the bottom.
    pub regions: Vec<Region>,
    pub warnings: Vec<String>,
}

/// A paint's colour rounded to 8 bits; `None` when a channel isn't finite.
pub fn rgb8(colour: [f32; 4]) -> Option<[u8; 3]> {
    let ch = |c: f32| c.is_finite().then(|| (f64::from(c).clamp(0.0, 1.0) * 255.0).round() as u8);
    Some([ch(colour[0])?, ch(colour[1])?, ch(colour[2])?])
}

/// Paints with less alpha than this are invisible and skipped.
const MIN_ALPHA: f32 = 0.5 / 255.0;

struct Item {
    colour: [u8; 3],
    path: BezPath,
    bbox: Rect,
}

#[derive(Default)]
struct Skipped {
    hidden: usize,
    translucent: usize,
    bad: usize,
    failed: usize,
    over_budget: bool,
}

impl Skipped {
    fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if self.hidden > 0 {
            w.push(format!("{} fully transparent shapes left out", self.hidden));
        }
        if self.translucent > 0 {
            w.push(format!("{} translucent shapes treated as opaque", self.translucent));
        }
        if self.bad > 0 {
            w.push(format!("{} shapes with invalid coordinates or colours left out", self.bad));
        }
        if self.failed > 0 {
            w.push(format!("{} shapes couldn't be combined and were left out", self.failed));
        }
        if self.over_budget {
            w.push(format!("the design has more than {MAX_POINTS} outline points; the shapes past that were left out"));
        }
        w
    }
}

/// One paint flattened at `flatten`, its rings simplified to `simplify` (0: not simplified) and
/// normalised. `None` when nothing is left; `Err` when the sweep fails.
fn normalise(path: &BezPath, flatten: f64, simplify: f64, budget: usize) -> Result<Option<(BezPath, usize)>, bool> {
    let Some(rs) = rings::rings(path, flatten, budget) else { return Err(true) };
    let rs: Vec<_> = rs.iter().map(|r| rings::simplify_ring(r, simplify)).filter(|r| r.len() >= 3).collect();
    let points: usize = rs.iter().map(Vec::len).sum();
    if rs.is_empty() {
        return Ok(None);
    }
    let norm = crate::ops::simplify(&rings::to_path(&rs)).map_err(|_| false)?;
    Ok((!norm.elements().is_empty()).then_some((norm, points)))
}

/// The paints bottom to top, normalised (see the module docs), with the skip counts.
fn prepare(paints: &[Paint], flatten: f64, simplify: f64) -> (Vec<Item>, Skipped) {
    let mut order: Vec<&Paint> = paints.iter().collect();
    // Stable: equal `order` keeps the input order.
    order.sort_by_key(|p| p.order);
    let mut skipped = Skipped::default();
    let mut items = Vec::new();
    let mut budget = MAX_POINTS;
    for p in order {
        let alpha = p.colour[3];
        if !alpha.is_finite() {
            skipped.bad += 1;
            continue;
        }
        if alpha < MIN_ALPHA {
            skipped.hidden += 1;
            continue;
        }
        let Some(colour) = rgb8(p.colour) else {
            skipped.bad += 1;
            continue;
        };
        if !crate::ops::finite(&p.path) {
            skipped.bad += 1;
            continue;
        }
        match normalise(&p.path, flatten, simplify, budget) {
            Ok(Some((path, points))) => {
                budget = budget.saturating_sub(points);
                if alpha < 1.0 - MIN_ALPHA {
                    skipped.translucent += 1;
                }
                let bbox = path.bounding_box();
                items.push(Item { colour, path, bbox });
            }
            Ok(None) => {}
            Err(true) => {
                skipped.over_budget = true;
                break;
            }
            Err(false) => skipped.failed += 1,
        }
    }
    (items, skipped)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The visible region of every colour in `paints` (see the module docs). Paths are read non-zero,
/// open subpaths closed. Curves are flattened at `flatten` (layer units, > 0) and every ring is then
/// simplified to `simplify` (0: kept as flattened). Paints are stacked by [`Paint::order`]
/// (0 = bottom; equal orders keep their input order). Fully transparent paints and paints with
/// invalid coordinates are left out, translucent ones count as opaque; each with a warning.
pub fn visible_regions(paints: &[Paint], flatten: f64, simplify: f64) -> Visible {
    let flatten = if flatten.is_finite() && flatten > 0.0 { flatten } else { 0.01 };
    let simplify = if simplify.is_finite() { simplify.max(0.0) } else { 0.0 };
    let (items, skipped) = prepare(paints, flatten, simplify);
    let mut warnings = skipped.warnings();
    // Runs of consecutive same-colour paints: (first item, end).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        match runs.last_mut() {
            Some((s, e)) if items.get(*s).map(|f| f.colour) == Some(it.colour) => *e = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    let mut regions: Vec<Region> = Vec::new();
    for it in &items {
        if !regions.iter().any(|r| r.colour == it.colour) {
            regions.push(Region { colour: it.colour, path: BezPath::new(), area: 0.0 });
        }
    }
    let mut failed = 0usize;
    for &(s, e) in runs.iter().rev() {
        let run = items.get(s..e).unwrap_or_default();
        let Some(first) = run.first() else { continue };
        let bbox = run.iter().fold(first.bbox, |b, it| b.union(it.bbox));
        let mut a = BezPath::new();
        for it in run {
            a.extend(it.path.iter());
        }
        let mut covered = BezPath::new();
        for it in items.get(e..).unwrap_or_default() {
            if overlaps(bbox, it.bbox) {
                covered.extend(it.path.iter());
            }
        }
        let Ok(piece) = crate::ops::boolean_step(&a, &covered, BoolOp::Subtract) else {
            failed += 1;
            continue;
        };
        if let Some(r) = regions.iter_mut().find(|r| r.colour == first.colour) {
            r.area += piece.area();
            r.path.extend(piece.iter());
        }
    }
    if failed > 0 {
        warnings.push(format!("{failed} groups of shapes couldn't be cut against the shapes above them and were left out"));
    }
    Visible { regions, warnings }
}
