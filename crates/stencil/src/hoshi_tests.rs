//! Registration holes (hoshi) on the sheet, through [`compute`].

use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{Point, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilTieMode};

use crate::{Paint, StencilOut, compute};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// 1 mm per unit, a 10 mm margin.
const B: Rect = Rect::new(0.0, 0.0, 100.0, 100.0);

fn design() -> Vec<Paint> {
    vec![
        Paint { path: B.to_path(0.01), colour: BLACK, order: 0 },
        Paint { path: Rect::new(30.0, 30.0, 70.0, 60.0).to_path(0.01), colour: WHITE, order: 1 },
    ]
}

fn params() -> Stencil {
    Stencil { width_mm: 100.0, margin_mm: 10.0, tie_mode: StencilTieMode::Gauze, ..Stencil::default() }
}

fn run(p: &Stencil) -> StencilOut {
    compute(&design(), p, B).unwrap()
}

/// The cut's contours whose centres are within 0.2 of `c`, and their area.
fn hole_at(out: &StencilOut, c: Point) -> Option<f64> {
    split_subpaths(&out.cut).iter().find(|s| (s.bounding_box().center() - c).hypot() < 0.2).map(|s| s.area())
}

#[test]
fn four_holes_in_the_margin_corners() {
    let out = run(&params());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let want = [Point::new(-5.0, -5.0), Point::new(105.0, -5.0), Point::new(105.0, 105.0), Point::new(-5.0, 105.0)];
    assert_eq!(out.hoshi, want);
    let disc = std::f64::consts::PI * 0.75 * 0.75;
    for c in want {
        let a = hole_at(&out, c).expect("a hole at each corner");
        assert!(a > 0.0 && (a - disc).abs() < disc * 0.02, "{c:?}: {a} vs {disc}");
    }
    assert_eq!(out.stats.cut_pieces, 5, "the design's cut-out and four holes");
    assert!(out.problems.is_empty(), "{:?}", out.problems);
    let sheet = out.sheet.area();
    assert!((out.paper.area() - (sheet - out.cut.area())).abs() < sheet * 1e-9);
    // Diameter follows the field and the physical scale (200 mm wide: 0.5 units per mm).
    let big = run(&Stencil { hoshi_diameter_mm: 4.0, width_mm: 200.0, ..params() });
    let a = hole_at(&big, big.hoshi[0]).unwrap();
    assert!((a - std::f64::consts::PI).abs() < 0.05, "{a}");
}

#[test]
fn off_or_no_room() {
    let off = run(&Stencil { hoshi: false, ..params() });
    assert!(off.hoshi.is_empty() && off.warnings.is_empty());
    assert_eq!(off.stats.cut_pieces, 1);
    let tight = run(&Stencil { margin_mm: 1.0, ..params() });
    assert!(tight.hoshi.is_empty());
    assert!(tight.warnings.iter().any(|w| w.contains("margin") && w.contains("registration")), "{:?}", tight.warnings);
    assert_eq!(tight.stats.cut_pieces, 1);
}

#[test]
fn holes_below_the_cut_width_warn_and_flag() {
    let out = run(&Stencil { hoshi_diameter_mm: 0.2, ..params() });
    assert_eq!(out.hoshi.len(), 4);
    assert!(out.warnings.iter().any(|w| w.contains("registration holes") && w.contains("minimum cut width")), "{:?}", out.warnings);
    // 0.03 mm² each: small cuts.
    assert_eq!(out.stats.problems_by_kind.get(crate::ProblemKind::SmallCut), 4, "{:?}", out.problems);
}

#[test]
fn a_hole_over_the_cut_merges_with_it() {
    // The design's cut-out runs into the margin corner where a hole goes.
    let mut paints = design();
    paints.push(Paint { path: Rect::new(-10.0, -10.0, 2.0, 2.0).to_path(0.01), colour: WHITE, order: 2 });
    let out = compute(&paints, &params(), B).unwrap();
    assert_eq!(out.hoshi.len(), 4);
    assert_eq!(out.stats.cut_pieces, 5, "the corner hole is inside the corner cut-out: {:?}", out.stats);
    let n = crate::ops::simplify(&out.cut).unwrap();
    assert!((n.area() - out.cut.area()).abs() < 1e-6, "plain contours");
}
