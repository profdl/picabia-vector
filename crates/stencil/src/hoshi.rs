//! Registration holes (hoshi): small round holes cut through the sheet so each print of the
//! stencil lines up with the last. On a sheet they sit in the margin at the design's four corners
//! (each corner pushed half the margin out diagonally: the middle of the margin's corner square);
//! on a repeat tile at the four corners of the cell (the lattice points), so a stamp's holes land on
//! the dots the previous stamp printed.

use crate::ops::BoolOp;
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape, Vec2};
use vectorcraft_modifiers::Stencil;

use crate::rings;

/// The most sides a hole's polygon gets.
const MAX_SIDES: usize = 256;
const MIN_SIDES: usize = 8;

/// The hole centres on a sheet: `design`'s corners pushed `margin / 2` out on both axes.
pub(crate) fn sheet_points(design: Rect, margin: f64) -> Vec<Point> {
    let d = design.abs();
    let h = margin / 2.0;
    vec![Point::new(d.x0 - h, d.y0 - h), Point::new(d.x1 + h, d.y0 - h), Point::new(d.x1 + h, d.y1 + h), Point::new(d.x0 - h, d.y1 + h)]
}

/// A circle of `radius` at `c` as a polygon within `tolerance` of it and of the same area (its
/// corners just outside the circle, its edges just inside), wound positively (a cut outline).
pub(crate) fn disc(c: Point, radius: f64, tolerance: f64) -> Vec<Point> {
    let t = tolerance.clamp(radius * 1e-4, radius);
    let step = 2.0 * (1.0 - t / radius).clamp(-1.0, 1.0).acos();
    let sides = if step.is_finite() && step > 0.0 { (std::f64::consts::TAU / step).ceil() as usize } else { MAX_SIDES };
    let n = sides.clamp(MIN_SIDES, MAX_SIDES);
    let wedge = std::f64::consts::TAU / n as f64;
    let radius = radius * (wedge / wedge.sin()).sqrt();
    let ring: Vec<Point> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            c + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    if crate::islands::ring_area(&ring) < 0.0 { ring.into_iter().rev().collect() } else { ring }
}

/// `cut` (plain contours) with holes `diameter` across at `points` added (flattened at
/// `tolerance`), and the holes alone. A union only when a hole meets the cut's bounds; otherwise
/// the discs are appended as they are (disjoint plain contours).
pub(crate) fn add(cut: &BezPath, points: &[Point], diameter: f64, tolerance: f64) -> Result<(BezPath, BezPath), crate::ops::PathOpsError> {
    let r = diameter / 2.0;
    if !(r.is_finite() && r > 0.0) || points.is_empty() {
        return Ok((cut.clone(), BezPath::new()));
    }
    let discs = rings::to_path(&points.iter().map(|&c| disc(c, r, tolerance)).collect::<Vec<_>>());
    let bb = cut.bounding_box();
    let meets = !cut.elements().is_empty() && points.iter().any(|c| c.x + r >= bb.x0 && c.x - r <= bb.x1 && c.y + r >= bb.y0 && c.y - r <= bb.y1);
    // Holes overlapping each other (a tiny cell) need the sweep too.
    let apart = points.iter().enumerate().all(|(i, p)| points.iter().skip(i + 1).all(|q| (*p - *q).hypot() > diameter));
    let out = if meets || !apart {
        crate::ops::boolean_step(cut, &discs, BoolOp::Union)?
    } else {
        let mut out = cut.clone();
        out.extend(discs.iter());
        out
    };
    Ok((out, discs))
}

/// Registration holes for the clamped params `p` at `mpu` mm per unit, centred on `points`, added
/// to `cut`: `(cut, centres, holes)`. None when `p.hoshi` is off, or when `room` (the margin, layer
/// units; a tile's holes sit on the cell corners and need none) is narrower than the diameter
/// (with a warning). A warning too when the holes are narrower than the minimum cut width.
pub(crate) fn register(
    p: &Stencil,
    cut: BezPath,
    points: Vec<Point>,
    room: f64,
    mpu: f64,
    warnings: &mut Vec<String>,
) -> (BezPath, Vec<Point>, BezPath) {
    if !p.hoshi {
        return (cut, Vec::new(), BezPath::new());
    }
    let d = p.hoshi_diameter_mm / mpu;
    if room < d {
        warnings.push(format!("no registration holes: the margin ({} mm) is narrower than their diameter ({} mm)", p.margin_mm, p.hoshi_diameter_mm));
        return (cut, Vec::new(), BezPath::new());
    }
    if p.hoshi_diameter_mm < p.min_cut_width_mm {
        warnings
            .push(format!("the registration holes ({} mm) are narrower than the minimum cut width ({} mm)", p.hoshi_diameter_mm, p.min_cut_width_mm));
    }
    let tolerance = if p.tolerance_mm > 0.0 { p.tolerance_mm / mpu / 2.0 } else { d * 1e-3 };
    match add(&cut, &points, d, tolerance) {
        Ok((c, holes)) => (c, points, holes),
        Err(_) => {
            warnings.push("couldn't cut the registration holes".into());
            (cut, Vec::new(), BezPath::new())
        }
    }
}
