//! Stage 3: clean-up of the cut region at physical size.
//!
//! The limits are **widths**, as a cutter's spec sheet gives them: a cut sliver narrower than
//! `minCutWidthMm` is removed by an opening of radius `minCutWidthMm / 2`, a paper bridge narrower
//! than `minBridgeMm` (paper between two cuts) is cut through by a closing of the cut of radius
//! `minBridgeMm / 2`. Areas filter whole contours: a cut piece (outer contour) under
//! `minCutAreaMm2` is dropped (a speck stays paper), a paper hole in the cut (hole contour) under
//! the same area is filled (it would fall out anyway). Anything nested in a dropped contour is
//! smaller still, so it goes too and the even-odd nesting of the rest holds.

use crate::ops::StrokeJoin;
use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{BezPath, Shape};

use crate::rings;

/// The cut region after clean-up, with what was removed.
#[derive(Debug, Default)]
pub(crate) struct Cleaned {
    pub cut: BezPath,
    pub dropped_specks: usize,
    pub filled_holes: usize,
    pub warnings: Vec<String>,
}

/// Clean-up limits in layer units (radii already halved from the widths).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub min_area: f64,
    pub open_radius: f64,
    pub close_radius: f64,
    pub tolerance: f64,
    /// `limitMode: fix`: the area filter, opening and closing run. `flag`: only the
    /// simplification runs, so every violation stays for the checks to report.
    pub fix: bool,
}

/// `path` (plain contours: outers positive, holes negative) without the contours enclosing less
/// than `min_area`; returns (path, outers dropped, holes filled).
pub(crate) fn drop_small(path: &BezPath, min_area: f64) -> (BezPath, usize, usize) {
    if min_area <= 0.0 {
        return (path.clone(), 0, 0);
    }
    let mut out = BezPath::new();
    let (mut specks, mut holes) = (0, 0);
    for sub in split_subpaths(path) {
        let a = sub.area();
        if a.abs() >= min_area {
            out.extend(sub.iter());
        } else if a > 0.0 {
            specks += 1;
        } else if a < 0.0 {
            holes += 1;
        }
    }
    (out, specks, holes)
}

/// Every contour of the polygonal region `path` simplified to `tolerance` and the result
/// normalised again (simplifying can make a contour cross itself). `None` if that sweep fails.
fn simplify_to(path: &BezPath, tolerance: f64) -> Option<BezPath> {
    crate::ops::simplify(&thin(path, tolerance)).ok()
}

/// The polygonal region `path` with every contour simplified to `tolerance`, no sweep (the
/// offsets normalise their input).
pub(crate) fn thin(path: &BezPath, tolerance: f64) -> BezPath {
    if tolerance <= 0.0 {
        return path.clone();
    }
    match rings::rings(path, tolerance * 0.1, usize::MAX) {
        Some(rs) => rings::to_path(&rs.iter().map(|r| rings::simplify_ring(r, tolerance)).filter(|r| r.len() >= 3).collect::<Vec<_>>()),
        None => path.clone(),
    }
}

/// Opening by `open` then closing by `close` (round joins), as offsets: shrink `open`, grow
/// `open`, grow `close`, shrink `close`. The two grows compose exactly (growing by a disc of
/// radius `a` then `b` is growing by one of `a + b`), so it is three offsets, not four. Between
/// offsets the region is thinned to `tolerance` (the round joins' fans are much finer than that).
pub(crate) fn open_close(cut: &BezPath, open: f64, close: f64, tolerance: f64) -> Result<BezPath, crate::ops::PathOpsError> {
    let steps: Vec<f64> = match (open > 0.0, close > 0.0) {
        (true, true) => vec![-open, open + close, -close],
        (true, false) => vec![-open, open],
        (false, true) => vec![close, -close],
        (false, false) => Vec::new(),
    };
    let mut cur = cut.clone();
    let last = steps.len().saturating_sub(1);
    for (i, d) in steps.into_iter().enumerate() {
        cur = crate::offset::offset(&cur, d, StrokeJoin::Round)?;
        if i < last {
            cur = thin(&cur, tolerance);
        }
    }
    Ok(cur)
}

/// Run the clean-up on `cut` (plain contours). If the opening and closing fail they are skipped
/// with a warning; the rest still runs. With `l.fix` off only the simplification runs.
pub(crate) fn clean(cut: BezPath, l: Limits) -> Cleaned {
    let mut c = Cleaned::default();
    let l = if l.fix { l } else { Limits { min_area: 0.0, open_radius: 0.0, close_radius: 0.0, ..l } };
    // Area first: specks dropped before the offsets cost nothing to offset.
    let (mut cut, s, h) = drop_small(&cut, l.min_area);
    c.dropped_specks += s;
    c.filled_holes += h;
    if !cut.elements().is_empty() && (l.open_radius > 0.0 || l.close_radius > 0.0) {
        match open_close(&cut, l.open_radius, l.close_radius, l.tolerance) {
            Ok(next) => cut = next,
            Err(_) => c.warnings.push("couldn't apply the minimum cut width and bridge; the cut is left without them".into()),
        }
    }
    let (next, s, h) = drop_small(&cut, l.min_area);
    cut = next;
    c.dropped_specks += s;
    c.filled_holes += h;
    if l.tolerance > 0.0 && !cut.elements().is_empty() {
        match simplify_to(&cut, l.tolerance) {
            Some(s) => cut = s,
            None => c.warnings.push("couldn't simplify the cut; it is left unsimplified".into()),
        }
    }
    c.cut = cut;
    c
}
