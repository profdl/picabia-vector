//! Islands, ties, gauze and the pattern lattice (stages 6–7), through [`compute`].

use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{BezPath, Circle, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilPatternType, StencilPatternWhere, StencilTieMode};

use crate::islands::Tree;
use crate::{Paint, StencilOut, compute};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// 1 mm per unit, a 10 mm margin.
const B: Rect = Rect::new(0.0, 0.0, 100.0, 100.0);

fn paint(path: BezPath, white: bool, order: u32) -> Paint {
    Paint { path, colour: if white { WHITE } else { BLACK }, order }
}

/// A black ground, then `shapes` alternating white (cut), black (paper), white... from the first.
fn nested(shapes: &[BezPath]) -> Vec<Paint> {
    let mut v = vec![paint(B.to_path(0.01), false, 0)];
    v.extend(shapes.iter().enumerate().map(|(i, s)| paint(s.clone(), i % 2 == 0, i as u32 + 1)));
    v
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.01)
}

/// No clean-up, no simplification, no registration holes.
fn raw() -> Stencil {
    Stencil {
        hoshi: false,
        width_mm: 100.0,
        margin_mm: 10.0,
        min_cut_area_mm2: 0.0,
        min_cut_width_mm: 0.0,
        min_bridge_mm: 0.0,
        tolerance_mm: 0.0,
        ..Stencil::default()
    }
}

fn run(paints: &[Paint], p: &Stencil) -> StencilOut {
    compute(paints, p, B).unwrap()
}

/// Islands still floating in `out`'s cut.
fn floating(out: &StencilOut) -> usize {
    Tree::new(&out.cut).unwrap().islands().len()
}

/// The ring 20..80 with the island 30..70 in it.
fn ring() -> Vec<Paint> {
    nested(&[rect(20.0, 20.0, 80.0, 80.0), rect(30.0, 30.0, 70.0, 70.0)])
}

fn bars(out: &StencilOut) -> Vec<Rect> {
    split_subpaths(&out.ties).iter().map(|s| s.bounding_box()).collect()
}

fn assert_held(out: &StencilOut) {
    assert_eq!(out.stats.untied, 0, "{:?} {:?}", out.stats, out.warnings);
    assert_eq!(floating(out), 0, "nothing floats");
    assert!(!out.warnings.iter().any(|w| w.contains("couldn't be tied")), "{:?}", out.warnings);
    let sheet = out.sheet.area();
    assert!((out.paper.area() - (sheet - out.cut.area())).abs() < sheet * 1e-9, "paper = sheet − cut");
}

#[test]
fn a_ring_with_an_island_gets_its_ties() {
    let p = raw();
    let out = run(&ring(), &p);
    assert_eq!(out.stats.islands, 1);
    assert_eq!(out.islands.len(), 1);
    let isl = out.islands[0];
    assert_eq!((isl.tied, isl.nested_depth, isl.pattern_held), (2, 0, false));
    assert!((isl.bounds.x0 - 30.0).abs() < 1e-6 && (isl.bounds.x1 - 70.0).abs() < 1e-6, "{:?}", isl.bounds);
    assert_eq!(out.stats.ties, 2);
    assert_held(&out);
    // Each bar is 1 mm wide across a 10 mm gap: 10 mm² of cut each becomes paper.
    assert!((out.cut.area() - (3600.0 - 1600.0 - 20.0)).abs() < 0.5, "{}", out.cut.area());
    for b in bars(&out) {
        let (long, short) = (b.width().max(b.height()), b.width().min(b.height()));
        assert!((short - 1.0).abs() < 1e-6 && (long - 11.0).abs() < 1e-6, "a 1 × (10 + 2 × 0.5) bar: {b:?}");
    }
    // The design with no island: nothing to tie.
    let out = run(&nested(&[rect(20.0, 20.0, 80.0, 80.0)]), &p);
    assert_eq!((out.stats.islands, out.stats.ties, out.stats.untied), (0, 0, 0));
    assert!(out.ties.elements().is_empty() && out.islands.is_empty());
}

#[test]
fn gauze_keeps_the_islands_and_flags_them() {
    let p = Stencil { tie_mode: StencilTieMode::Gauze, ..raw() };
    let out = run(&ring(), &p);
    assert_eq!((out.stats.islands, out.stats.ties, out.stats.untied), (1, 0, 1));
    assert_eq!(out.islands[0].tied, 0);
    assert!(out.ties.elements().is_empty());
    assert!((out.cut.area() - 2000.0).abs() < 1e-6, "the cut is untouched");
    assert!(out.warnings.iter().any(|w| w.contains("gauze")), "{:?}", out.warnings);
    assert_eq!(floating(&out), 1);
}

#[test]
fn nested_islands_three_levels() {
    let shapes: Vec<BezPath> = (0..6).map(|i| f64::from(i) * 5.0).map(|d| rect(10.0 + d, 10.0 + d, 90.0 - d, 90.0 - d)).collect();
    let out = run(&nested(&shapes), &raw());
    assert_eq!(out.stats.islands, 3);
    let depths: Vec<u32> = out.islands.iter().map(|i| i.nested_depth).collect();
    assert_eq!(depths, [0, 1, 2], "outermost first");
    assert!(out.islands.iter().all(|i| i.tied == 2), "{:?}", out.islands);
    assert_eq!(out.stats.ties, 6);
    assert_held(&out);
}

#[test]
fn islands_touching_at_a_corner() {
    let p = raw();
    let out = run(
        &nested(&[rect(10.0, 10.0, 90.0, 90.0), rect(20.0, 20.0, 50.0, 50.0)])
            .into_iter()
            .chain([paint(rect(50.0, 50.0, 80.0, 80.0), false, 9)])
            .collect::<Vec<_>>(),
        &p,
    );
    assert!(out.stats.islands >= 1, "{:?}", out.stats);
    assert_held(&out);
}

#[test]
fn an_island_by_the_sheet_edge() {
    // No margin, and the cut-out runs off the layer on two sides: the bars must go to the paper
    // on the other two sides, never off the sheet.
    let p = Stencil { margin_mm: 0.0, ..raw() };
    let out = run(&nested(&[rect(-5.0, -5.0, 30.0, 30.0), rect(2.0, 2.0, 25.0, 25.0)]), &p);
    assert_eq!(out.stats.islands, 1);
    assert_held(&out);
    for b in bars(&out) {
        assert!(b.x0 >= 0.0 && b.y0 >= 0.0, "on the sheet: {b:?}");
        assert!(b.x1 > 25.0 || b.y1 > 25.0, "to the paper right or below: {b:?}");
    }
}

#[test]
fn a_symmetric_island_ties_the_same_for_a_seed() {
    let c = |r: f64| Circle::new((50.0, 50.0), r).to_path(0.01);
    let paints = nested(&[c(30.0), c(20.0)]);
    let at = |seed: u32| run(&paints, &Stencil { tie_seed: seed, ..raw() });
    let a = at(1);
    assert_held(&a);
    assert_eq!(a.ties, at(1).ties, "same seed, same ties");
    assert!((2..12).any(|s| at(s).ties != a.ties), "another seed picks other equally short gaps");
}

#[test]
fn the_angle_is_preferred() {
    for (angle, horizontal) in [(Some(0.0), true), (Some(180.0), true), (Some(90.0), false), (Some(270.0), false)] {
        let out = run(&ring(), &Stencil { tie_angle: angle, ..raw() });
        assert_held(&out);
        let bs = bars(&out);
        assert_eq!(bs.len(), 2);
        for b in &bs {
            assert_eq!(b.width() > b.height(), horizontal, "{angle:?}: {b:?}");
        }
        // Spread: one each side.
        let (a, b) = (bs[0].center(), bs[1].center());
        assert!((a - b).hypot() > 45.0, "{angle:?}: {a:?} {b:?}");
    }
    // Diagonal: still across the gaps (the only ones there are), here a single bar.
    let out = run(&ring(), &Stencil { tie_angle: Some(45.0), ties_per_island: 1, ..raw() });
    assert_held(&out);
}

#[test]
fn spread_keeps_ties_apart() {
    // The island sits 3 from the ring's left side, 10 from the others.
    let paints = nested(&[rect(20.0, 20.0, 80.0, 80.0), rect(23.0, 30.0, 70.0, 70.0)]);
    let close = run(&paints, &Stencil { tie_spread: false, ..raw() });
    assert_held(&close);
    let bs = bars(&close);
    assert!(bs.iter().all(|b| b.x1 < 24.0), "both on the shortest gaps: {bs:?}");
    let spread = run(&paints, &raw());
    assert_held(&spread);
    let bs = bars(&spread);
    assert!((bs[0].center() - bs[1].center()).hypot() >= 30.0, "kept apart: {bs:?}");
}

#[test]
fn ties_per_island_one_to_eight() {
    let c = |r: f64| Circle::new((50.0, 50.0), r).to_path(0.01);
    let paints = nested(&[c(40.0), c(30.0)]);
    for n in 1..=8 {
        for spread in [true, false] {
            let out = run(&paints, &Stencil { ties_per_island: n, tie_spread: spread, ..raw() });
            assert_eq!(out.islands[0].tied, n, "{n} {spread}");
            assert_eq!(out.stats.ties, n as usize);
            assert_held(&out);
        }
    }
}

#[test]
fn junk_params_never_break() {
    let base = ring();
    let junk = [
        Stencil { tie_width_mm: f64::NAN, ties_per_island: 0, tie_angle: Some(f64::INFINITY), tie_seed: u32::MAX, ..raw() },
        Stencil { tie_width_mm: 1e9, ties_per_island: 1000, ..raw() },
        Stencil { tie_width_mm: -3.0, tie_angle: Some(-1e300), ..raw() },
        Stencil { tie_mode: StencilTieMode::Pattern, pattern_width_mm: f64::NAN, pattern_spacing_mm: 0.0, pattern_angle: f64::NEG_INFINITY, ..raw() },
        Stencil { tie_mode: StencilTieMode::Pattern, pattern_width_mm: 50.0, pattern_spacing_mm: 1e9, ..raw() },
        Stencil { tie_mode: StencilTieMode::Pattern, width_mm: 10_000.0, pattern_width_mm: 0.05, pattern_spacing_mm: 0.5, ..raw() },
    ];
    for p in &junk {
        let out = run(&base, p);
        assert_eq!(out.stats.islands, 1, "{p:?}");
        // A 50 mm bar can't fit a 10 mm gap without touching other contours: flagged, not broken.
        assert_eq!(out.stats.untied, floating(&out), "{p:?}");
    }
    // Too fine a lattice is left out with a warning; the fallback ties still hold the island.
    let out = run(&base, &junk[5]);
    assert!(out.warnings.iter().any(|w| w.contains("too fine")), "{:?}", out.warnings);
    assert_held(&out);
}

#[test]
fn pattern_grid_holds_islands_it_crosses() {
    let p = Stencil { tie_mode: StencilTieMode::Pattern, ..raw() };
    let out = run(&ring(), &p);
    assert_eq!((out.stats.islands, out.stats.pattern_held, out.stats.ties), (1, 1, 0), "{:?}", out.stats);
    assert!(out.islands[0].pattern_held && out.islands[0].tied == 0);
    assert_held(&out);
    assert!(!out.ties.elements().is_empty(), "the lattice in the paper is in the overlay");
    // The lattice turned some cut into paper, only inside the cut.
    assert!(out.cut.area() < 2000.0 - 10.0);
    assert!(out.ties.bounding_box().x0 >= 20.0 - 1e-6 && out.ties.bounding_box().x1 <= 80.0 + 1e-6, "{:?}", out.ties.bounding_box());
    // Thin bars warn.
    let out = run(&ring(), &Stencil { min_bridge_mm: 1.0, ..p });
    assert!(out.warnings.iter().any(|w| w.contains("pattern bars")), "{:?}", out.warnings);
}

#[test]
fn pattern_horizontal_vertical_and_angle() {
    let w = 0.6;
    for (kind, angle, flat) in [
        (StencilPatternType::Horizontal, 0.0, true),
        (StencilPatternType::Vertical, 0.0, false),
        (StencilPatternType::Horizontal, 90.0, false),
        (StencilPatternType::Vertical, 270.0, true),
    ] {
        let out = run(&ring(), &Stencil { tie_mode: StencilTieMode::Pattern, pattern_type: kind, pattern_angle: angle, ..raw() });
        assert_held(&out);
        assert_eq!(out.stats.pattern_held, 1);
        for b in bars(&out) {
            let thin = if flat { b.height() } else { b.width() };
            assert!(thin <= w + 1e-6, "{kind:?} {angle}: {b:?}");
        }
    }
    // A turned lattice: pieces are neither flat nor upright, and still hold the island.
    let out =
        run(&ring(), &Stencil { tie_mode: StencilTieMode::Pattern, pattern_type: StencilPatternType::Horizontal, pattern_angle: 30.0, ..raw() });
    assert_held(&out);
    assert_eq!(out.stats.pattern_held, 1);
    assert!(bars(&out).iter().any(|b| b.height() > 2.0 && b.width() > 2.0));
}

#[test]
fn pattern_falls_back_to_ties_for_islands_it_misses() {
    // The lattice runs every 8 from the sheet corner (-10, -10): lines at 38 and 46 miss this
    // small ring 39.5..45.5 and its island.
    let small = nested(&[rect(39.5, 39.5, 45.5, 45.5), rect(41.5, 41.5, 43.5, 43.5)]);
    let out = run(&small, &Stencil { tie_mode: StencilTieMode::Pattern, ..raw() });
    assert_eq!((out.stats.islands, out.stats.pattern_held, out.stats.ties), (1, 0, 2), "{:?}", out.stats);
    assert_eq!(out.islands[0].tied, 2);
    assert_held(&out);
    // A finer lattice crosses it.
    let out = run(&small, &Stencil { tie_mode: StencilTieMode::Pattern, pattern_spacing_mm: 1.5, pattern_width_mm: 0.3, ..raw() });
    assert_eq!((out.stats.pattern_held, out.stats.ties), (1, 0), "{:?}", out.stats);
    assert_held(&out);
}

#[test]
fn pattern_only_where_islands_are() {
    // The ring with its island, and a plain cut-out with none.
    let mut paints = ring();
    paints.push(paint(rect(85.0, 5.0, 95.0, 95.0), true, 9));
    let everywhere = run(&paints, &Stencil { tie_mode: StencilTieMode::Pattern, ..raw() });
    let islands = run(&paints, &Stencil { tie_mode: StencilTieMode::Pattern, pattern_where: StencilPatternWhere::Islands, ..raw() });
    assert_held(&everywhere);
    assert_held(&islands);
    let plain =
        |out: &StencilOut| -> f64 { split_subpaths(&out.cut).iter().filter(|s| s.bounding_box().x0 >= 84.0).map(|s| s.area()).sum::<f64>().abs() };
    assert!((plain(&islands) - 900.0).abs() < 1e-6, "untouched: {}", plain(&islands));
    assert!(plain(&everywhere) < 900.0 - 5.0, "crossed: {}", plain(&everywhere));
    assert_eq!(islands.stats.pattern_held, 1);
}

fn sq(x0: f64, y0: f64, x1: f64, y1: f64, ccw: bool) -> Vec<vectorcraft_geom::kurbo::Point> {
    use vectorcraft_geom::kurbo::Point;
    let v = vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];
    if ccw { v } else { v.into_iter().rev().collect() }
}

#[test]
fn the_contour_tree() {
    // A cut outline with two holes touching at a corner, a cut inside one of them, and an island
    // in that cut's hole; a separate cut-out with no hole. Orientation doesn't matter.
    let rings = vec![
        sq(0.0, 0.0, 10.0, 10.0, true),
        sq(1.0, 1.0, 5.0, 5.0, false),
        sq(5.0, 5.0, 9.0, 9.0, true),
        sq(6.0, 6.0, 8.5, 8.5, false),
        sq(7.0, 7.0, 8.0, 8.0, true),
        sq(20.0, 0.0, 30.0, 10.0, true),
    ];
    let t = Tree::of_rings(rings);
    assert_eq!(t.parent, [None, Some(0), Some(0), Some(2), Some(3), None]);
    assert_eq!(t.depth, [0, 1, 1, 2, 3, 0]);
    assert_eq!(t.islands(), [1, 2, 4], "the holes, outermost first; the sheet and the cut-outs aren't islands");
    assert_eq!([1, 2, 4].map(|i| t.nested_depth(i)), [0, 0, 1]);
    // Many islands: one ray each.
    let mut rings = vec![sq(0.0, 0.0, 1000.0, 1000.0, true)];
    for i in 0..60 {
        for j in 0..60 {
            let (x, y) = (f64::from(i) * 16.0 + 10.0, f64::from(j) * 16.0 + 10.0);
            rings.push(sq(x, y, x + 8.0, y + 8.0, false));
        }
    }
    let t = Tree::of_rings(rings);
    assert_eq!(t.islands().len(), 3600);
    assert!(t.parent.iter().skip(1).all(|&p| p == Some(0)));
}

#[test]
fn many_islands_all_tied() {
    let mut shapes = vec![paint(B.to_path(0.01), false, 0), paint(rect(5.0, 5.0, 95.0, 95.0), true, 1)];
    for i in 0..8 {
        for j in 0..8 {
            let (x, y) = (8.0 + f64::from(i) * 11.0, 8.0 + f64::from(j) * 11.0);
            shapes.push(paint(rect(x, y, x + 6.0, y + 6.0), false, 2));
        }
    }
    let out = run(&shapes, &raw());
    assert_eq!(out.stats.islands, 64);
    assert_held(&out);
    assert!(out.islands.iter().all(|i| i.tied >= 1), "{:?}", out.islands.iter().map(|i| i.tied).collect::<Vec<_>>());
}
