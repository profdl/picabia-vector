use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{BezPath, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilColourMode, StencilTieMode};

use crate::{MAX_PAINTS, Paint, StencilError, StencilOut, compute};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// 100 units wide at 100 mm: 1 mm per unit.
const B: Rect = Rect::new(0.0, 0.0, 100.0, 100.0);

fn rect(r: Rect, colour: [f32; 4], order: u32) -> Paint {
    Paint { path: r.to_path(0.01), colour, order }
}

/// A black ground with `white` shapes on top.
fn on_black(white: &[Rect]) -> Vec<Paint> {
    let mut v = vec![rect(B, BLACK, 0)];
    v.extend(white.iter().enumerate().map(|(i, r)| rect(*r, WHITE, i as u32 + 1)));
    v
}

/// Gauze: these tests look at the cut before ties change it (`hold_tests` has the ties); no
/// registration holes (`hoshi_tests`).
fn params() -> Stencil {
    Stencil { width_mm: 100.0, margin_mm: 10.0, tie_mode: StencilTieMode::Gauze, hoshi: false, ..Stencil::default() }
}

/// No clean-up, no simplification.
fn raw() -> Stencil {
    Stencil { min_cut_area_mm2: 0.0, min_cut_width_mm: 0.0, min_bridge_mm: 0.0, tolerance_mm: 0.0, ..params() }
}

fn run(paints: &[Paint], p: &Stencil) -> StencilOut {
    compute(paints, p, B).unwrap()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn assert_paper_is_sheet_minus_cut(out: &StencilOut) {
    let sheet = out.sheet.area();
    let (cut, paper) = (out.cut.area(), out.paper.area());
    assert!(near(paper, sheet - cut, 1e-6 * sheet), "paper {paper} = sheet {sheet} − cut {cut}");
    // Normalising the paper (non-zero) changes nothing: even-odd and non-zero agree.
    let norm = crate::ops::simplify(&out.paper).unwrap();
    assert!(near(norm.area(), paper, 1e-6 * sheet));
}

#[test]
fn a_light_square_on_dark_ground_is_cut_out() {
    let out = run(&on_black(&[Rect::new(20.0, 20.0, 40.0, 50.0)]), &raw());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.mm_per_unit, 1.0);
    assert_eq!(out.sheet, Rect::new(-10.0, -10.0, 110.0, 110.0), "margin in layer units");
    assert!(near(out.cut.area(), 600.0, 1e-6));
    assert_eq!(out.stats.cut_pieces, 1);
    assert!(near(out.stats.cut_length_mm, 100.0, 1e-6));
    assert!(near(out.stats.area_cut_mm2, 600.0, 1e-6));
    assert_paper_is_sheet_minus_cut(&out);
    let sw: Vec<(&str, bool)> = out.colours.iter().map(|c| (c.hex.as_str(), c.cut)).collect();
    assert_eq!(sw, [("#000000", false), ("#ffffff", true)]);
    assert!(near(out.colours[0].area, 10_000.0 - 600.0, 1e-6));
}

#[test]
fn physical_scale_converts_lengths_and_areas() {
    // The same design at 200 mm: lengths double, areas quadruple, the margin is 10 mm = 5 units.
    let out = run(&on_black(&[Rect::new(20.0, 20.0, 40.0, 50.0)]), &Stencil { width_mm: 200.0, ..raw() });
    assert_eq!(out.mm_per_unit, 2.0);
    assert_eq!(out.sheet, Rect::new(-5.0, -5.0, 105.0, 105.0));
    assert!(near(out.stats.cut_length_mm, 200.0, 1e-6));
    assert!(near(out.stats.area_cut_mm2, 2400.0, 1e-6));
    assert!(near(out.colours[1].area, 2400.0, 1e-6));
}

#[test]
fn invert_and_pick_choose_the_other_side() {
    let paints = on_black(&[Rect::new(20.0, 20.0, 40.0, 50.0)]);
    let out = run(&paints, &Stencil { invert: true, ..raw() });
    assert!(near(out.cut.area(), 10_000.0 - 600.0, 1e-6), "the ground is cut, the square stays");
    assert_eq!(out.stats.cut_pieces, 1);
    assert_eq!(split_subpaths(&out.cut).len(), 2, "one outer, one hole");
    assert_paper_is_sheet_minus_cut(&out);
    let pick = Stencil { colour_mode: StencilColourMode::Pick, cut_colours: vec!["#000000".into()], ..raw() };
    assert!(near(run(&paints, &pick).cut.area(), 9400.0, 1e-6));
    let none = run(&paints, &Stencil { colour_mode: StencilColourMode::Pick, ..raw() });
    assert!(none.cut.elements().is_empty());
    assert!(none.warnings.iter().any(|w| w.contains("nothing is cut")), "{:?}", none.warnings);
    assert!(near(none.paper.area(), none.sheet.area(), 1e-9));
}

#[test]
fn specks_are_dropped_and_tiny_paper_holes_filled() {
    // A 0.5 × 0.5 mm white speck (0.25 mm² < 0.5) and a 20 mm square with a 0.5 mm black dot.
    let mut paints = on_black(&[Rect::new(5.0, 5.0, 5.5, 5.5), Rect::new(20.0, 20.0, 40.0, 40.0)]);
    paints.push(rect(Rect::new(30.0, 30.0, 30.5, 30.5), BLACK, 9));
    let p = Stencil { min_cut_width_mm: 0.0, min_bridge_mm: 0.0, tolerance_mm: 0.0, ..params() };
    let out = run(&paints, &p);
    assert_eq!((out.stats.dropped_specks, out.stats.filled_holes), (1, 1), "{:?}", out.stats);
    assert_eq!(out.stats.cut_pieces, 1);
    assert!(near(out.cut.area(), 400.0, 1e-6), "the hole is filled: {}", out.cut.area());
    // With no area limit both stay.
    let out = run(&paints, &raw());
    assert_eq!((out.stats.dropped_specks, out.stats.filled_holes, out.stats.cut_pieces), (0, 0, 2));
    assert!(near(out.cut.area(), 400.0 - 0.25 + 0.25, 1e-6));
}

#[test]
fn opening_removes_cuts_narrower_than_the_minimum_width() {
    // A 0.2 mm wide, 30 mm long slit (6 mm², above the area limit) and a 10 mm square.
    let paints = on_black(&[Rect::new(10.0, 10.0, 10.2, 40.0), Rect::new(50.0, 50.0, 60.0, 60.0)]);
    let p = Stencil { min_bridge_mm: 0.0, tolerance_mm: 0.0, ..params() };
    let out = run(&paints, &p);
    assert_eq!(out.stats.cut_pieces, 1, "the slit is gone");
    assert!(out.cut.bounding_box().x0 > 49.0);
    // The square keeps its size (corners rounded by the 0.15 mm radius only).
    assert!(near(out.cut.area(), 100.0, 0.1), "{}", out.cut.area());
    // 0.4 mm: wider than the limit, kept.
    let out = run(&on_black(&[Rect::new(10.0, 10.0, 10.4, 40.0)]), &p);
    assert_eq!(out.stats.cut_pieces, 1);
    // No limit: the 0.2 mm slit stays.
    assert_eq!(run(&paints, &Stencil { min_cut_width_mm: 0.0, ..p }).stats.cut_pieces, 2);
}

#[test]
fn closing_cuts_through_bridges_narrower_than_the_minimum() {
    // Two squares 0.3 mm apart (the paper between them is a 0.3 mm bridge < 0.5).
    let paints = on_black(&[Rect::new(10.0, 10.0, 20.0, 20.0), Rect::new(20.3, 10.0, 30.3, 20.0)]);
    let p = Stencil { min_cut_width_mm: 0.0, tolerance_mm: 0.0, ..params() };
    let out = run(&paints, &p);
    assert_eq!(out.stats.cut_pieces, 1, "the bridge is cut through");
    assert!(near(out.cut.area(), 203.0, 0.05), "{}", out.cut.area());
    // A 1 mm bridge stays.
    let out = run(&on_black(&[Rect::new(10.0, 10.0, 20.0, 20.0), Rect::new(21.0, 10.0, 31.0, 20.0)]), &p);
    assert_eq!(out.stats.cut_pieces, 2);
    assert_paper_is_sheet_minus_cut(&out);
}

#[test]
fn simplification_stays_within_the_tolerance() {
    // A circle of radius 20 mm drawn with a fine flattening.
    let circle = Paint { path: vectorcraft_geom::kurbo::Circle::new((50.0, 50.0), 20.0).to_path(1e-4), colour: WHITE, order: 1 };
    let paints = [rect(B, BLACK, 0), circle];
    let fine = run(&paints, &Stencil { tolerance_mm: 0.0, ..params() });
    let coarse = run(&paints, &Stencil { tolerance_mm: 0.5, ..params() });
    let (nf, nc) = (fine.cut.elements().len(), coarse.cut.elements().len());
    assert!(nc * 4 < nf, "{nf} → {nc}");
    let exact = std::f64::consts::PI * 400.0;
    // Area error within perimeter × tolerance.
    assert!(near(coarse.cut.area(), exact, 2.0 * std::f64::consts::PI * 20.0 * 0.5), "{}", coarse.cut.area());
    assert!(near(fine.cut.area(), exact, 0.5), "{}", fine.cut.area());
}

#[test]
fn cuts_outside_the_sheet_are_clipped() {
    let paints = [rect(Rect::new(-50.0, 40.0, 50.0, 60.0), WHITE, 0)];
    let out = run(&paints, &raw());
    assert!(out.cut.bounding_box().x0 >= out.sheet.x0 - 1e-9);
    assert!(near(out.cut.area(), 60.0 * 20.0, 1e-6));
    assert_paper_is_sheet_minus_cut(&out);
}

#[test]
fn bad_bounds_too_many_paints_and_empty_input() {
    let paints = on_black(&[]);
    for b in [Rect::new(0.0, 0.0, 0.0, 10.0), Rect::new(0.0, 0.0, f64::NAN, 10.0), Rect::new(f64::NEG_INFINITY, 0.0, 1.0, 1.0)] {
        assert_eq!(compute(&paints, &params(), b), Err(StencilError::BadBounds), "{b:?}");
    }
    let many: Vec<Paint> = (0..=MAX_PAINTS).map(|i| rect(Rect::new(0.0, 0.0, 1.0, 1.0), WHITE, i as u32)).collect();
    let out = run(&many, &params());
    assert!(out.cut.elements().is_empty() && out.warnings.iter().any(|w| w.contains("at most")), "{:?}", out.warnings);
    assert!(near(out.paper.area(), out.sheet.area(), 1e-9));
    let out = run(&[], &params());
    assert!(out.cut.elements().is_empty() && out.colours.is_empty());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // Out-of-range params are clamped, with their warnings passed on.
    let out = run(&paints, &Stencil { threshold: 5.0, tolerance_mm: f64::NAN, ..params() });
    assert_eq!(out.warnings.iter().filter(|w| w.contains("threshold") || w.contains("toleranceMm")).count(), 2, "{:?}", out.warnings);
}

#[test]
fn the_cleanup_can_remove_everything() {
    let out = run(&on_black(&[Rect::new(5.0, 5.0, 5.5, 5.5)]), &params());
    assert!(out.cut.elements().is_empty());
    assert_eq!(out.stats.dropped_specks, 1);
    assert!(out.warnings.iter().any(|w| w.contains("nothing is left")), "{:?}", out.warnings);
    assert_eq!(out.paper, {
        let mut p = BezPath::new();
        p.extend(out.sheet.to_path(0.1).iter());
        p
    });
}
