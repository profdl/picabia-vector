//! Repeat-aware islands and ties, and the repeat tile, through [`compute_repeat`].

use crate::ops::BoolOp;
use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape, Vec2};
use vectorcraft_modifiers::{Repeat, Stencil, StencilOutput, StencilTieMode};

use crate::{Paint, StencilOut, compute, compute_repeat};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// No clean-up, no simplification, no registration holes; 1 mm per unit for a 110-unit design.
fn raw(width_mm: f64) -> Stencil {
    Stencil {
        width_mm,
        margin_mm: 10.0,
        min_cut_area_mm2: 0.0,
        min_cut_width_mm: 0.0,
        min_bridge_mm: 0.0,
        tolerance_mm: 0.0,
        hoshi: false,
        ..Stencil::default()
    }
}

/// The motif at `o`: a white ring 10..26 with a black island 14..22 inside (so in a 20-unit cell
/// the island crosses the cell's edge at 20).
fn motif(o: Vec2, order: u32) -> [Paint; 2] {
    let r = |a: f64, b: f64| Rect::new(o.x + a, o.y + a, o.x + b, o.y + b).to_path(0.01);
    [Paint { path: r(10.0, 26.0), colour: WHITE, order }, Paint { path: r(14.0, 22.0), colour: BLACK, order: order + 1 }]
}

/// The motif at every `i·a + j·b` (`i < n`, `j < m`) on a black ground: the paints and the
/// design's bounds (the ground).
fn repeated(a: Vec2, b: Vec2, n: i32, m: i32) -> (Vec<Paint>, Rect) {
    let mut paints = Vec::new();
    for i in 0..n {
        for j in 0..m {
            let o = a * f64::from(i) + b * f64::from(j);
            paints.extend(motif(o, 1 + 2 * paints.len() as u32));
        }
    }
    let bb = paints.iter().fold(Rect::new(10.0, 10.0, 26.0, 26.0), |r, p| r.union(p.path.bounding_box())).inflate(4.0, 4.0);
    paints.insert(0, Paint { path: bb.to_path(0.01), colour: BLACK, order: 0 });
    (paints, bb)
}

fn lattice(a: Vec2, b: Vec2) -> Repeat {
    Repeat { origin: Point::ZERO, a, b }
}

fn bars(out: &StencilOut) -> Vec<Rect> {
    split_subpaths(&out.ties).iter().map(|s| s.bounding_box()).collect()
}

/// Each copy's bars (the copy whose ring holds the bar's centre), moved back to the first copy.
fn per_copy(out: &StencilOut, a: Vec2, b: Vec2, n: i32, m: i32) -> Vec<Vec<Rect>> {
    let mut groups = Vec::new();
    for i in 0..n {
        for j in 0..m {
            let o = a * f64::from(i) + b * f64::from(j);
            let ring = Rect::new(o.x + 10.0, o.y + 10.0, o.x + 26.0, o.y + 26.0);
            let mut g: Vec<Rect> = bars(out).into_iter().filter(|r| ring.contains(r.center())).map(|r| r - o).collect();
            g.sort_by(|p, q| p.x0.total_cmp(&q.x0).then(p.y0.total_cmp(&q.y0)));
            groups.push(g);
        }
    }
    groups
}

fn same_rects(a: &[Rect], b: &[Rect]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(p, q)| (p.x0 - q.x0).abs() < 1e-6 && (p.y0 - q.y0).abs() < 1e-6 && (p.x1 - q.x1).abs() < 1e-6 && (p.y1 - q.y1).abs() < 1e-6)
}

fn assert_tied_alike(out: &StencilOut, groups: &[Vec<Rect>]) {
    assert_eq!(out.stats.untied, 0, "{:?} {:?}", out.stats, out.warnings);
    let first = &groups[0];
    assert!(!first.is_empty());
    for (k, g) in groups.iter().enumerate() {
        assert!(same_rects(g, first), "copy {k}: {g:?} vs {first:?}");
    }
}

#[test]
fn an_island_across_the_cell_edge_is_tied_the_same_in_every_cell() {
    let (a, b) = (Vec2::new(20.0, 0.0), Vec2::new(0.0, 20.0));
    let (paints, bounds) = repeated(a, b, 5, 5);
    let p = raw(bounds.width());
    let out = compute_repeat(&paints, &p, bounds, Some(lattice(a, b))).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!((out.stats.islands, out.stats.repeat_islands), (25, 1), "{:?}", out.stats);
    assert_eq!(out.stats.ties, 50);
    assert!(out.islands.iter().all(|i| i.tied == 2), "{:?}", out.islands);
    assert_tied_alike(&out, &per_copy(&out, a, b, 5, 5));
    // The cell nearest the centre (52, 52): [40, 60]².
    let cell = out.cell.as_ref().unwrap().bounding_box();
    assert_eq!(cell, Rect::new(40.0, 40.0, 60.0, 60.0));
    assert!(out.tile.is_none());
    // Deterministic.
    assert_eq!(compute_repeat(&paints, &p, bounds, Some(lattice(a, b))).unwrap(), out);
    // Without the repeat each copy is tied on its own (seeded by its number), not as a translate.
    let plain = compute(&paints, &p, bounds).unwrap();
    assert_eq!((plain.stats.islands, plain.stats.repeat_islands, plain.stats.untied), (25, 0, 0));
    assert!(plain.cell.is_none());
    let g = per_copy(&plain, a, b, 5, 5);
    assert!(g.iter().filter(|x| !same_rects(x, &g[0])).count() > 10, "copies tied each their own way");
}

#[test]
fn a_one_dimensional_repeat() {
    // A grid of one row: b = 0.
    let a = Vec2::new(20.0, 0.0);
    let (paints, bounds) = repeated(a, Vec2::ZERO, 5, 1);
    let out = compute_repeat(&paints, &raw(bounds.width()), bounds, Some(lattice(a, Vec2::ZERO))).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!((out.stats.islands, out.stats.repeat_islands), (5, 1), "{:?}", out.stats);
    assert_tied_alike(&out, &per_copy(&out, a, Vec2::ZERO, 5, 1));
    // The strip's cell spans the design across the repeat.
    let cell = out.cell.as_ref().unwrap().bounding_box();
    assert!((cell.width() - 20.0).abs() < 1e-9 && (cell.y0 - bounds.y0).abs() < 1e-9 && (cell.y1 - bounds.y1).abs() < 1e-9, "{cell:?}");
    // The vector may be either one.
    let col = compute_repeat(&paints, &raw(bounds.width()), bounds, Some(lattice(Vec2::ZERO, a))).unwrap();
    assert_eq!(col.stats.repeat_islands, 1);
    assert_eq!(col.ties, out.ties);
}

#[test]
fn an_oblique_lattice() {
    let (a, b) = (Vec2::new(20.0, 0.0), Vec2::new(10.0, 20.0));
    let (paints, bounds) = repeated(a, b, 4, 4);
    let out = compute_repeat(&paints, &raw(bounds.width()), bounds, Some(lattice(a, b))).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!((out.stats.islands, out.stats.repeat_islands), (16, 1), "{:?}", out.stats);
    assert_tied_alike(&out, &per_copy(&out, a, b, 4, 4));
    let cell = out.cell.as_ref().unwrap();
    assert!((cell.area().abs() - 400.0).abs() < 1e-6, "a parallelogram of |a × b|");
}

#[test]
fn a_degenerate_lattice_is_ignored_with_a_warning() {
    let (a, b) = (Vec2::new(20.0, 0.0), Vec2::new(0.0, 20.0));
    let (paints, bounds) = repeated(a, b, 3, 3);
    let p = raw(bounds.width());
    let plain = compute(&paints, &p, bounds).unwrap();
    for r in [
        lattice(a, Vec2::new(40.0, 0.0)),
        lattice(Vec2::ZERO, Vec2::ZERO),
        lattice(a, Vec2::new(f64::NAN, 1.0)),
        Repeat { origin: Point::new(f64::INFINITY, 0.0), a, b },
        lattice(Vec2::new(0.001, 0.0), Vec2::new(0.0, 0.001)),
    ] {
        let out = compute_repeat(&paints, &p, bounds, Some(r)).unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("ignores the repeat")), "{r:?}: {:?}", out.warnings);
        assert_eq!((out.cut.clone(), out.stats.repeat_islands, out.cell.clone()), (plain.cut.clone(), 0, None), "{r:?}");
    }
    // Pattern mode isn't repeat-aware; gauze only counts.
    let pat = compute_repeat(&paints, &Stencil { tie_mode: StencilTieMode::Pattern, ..p.clone() }, bounds, Some(lattice(a, b))).unwrap();
    assert!(pat.warnings.iter().any(|w| w.contains("isn't repeat-aware")), "{:?}", pat.warnings);
    let gauze = compute_repeat(&paints, &Stencil { tie_mode: StencilTieMode::Gauze, ..p }, bounds, Some(lattice(a, b))).unwrap();
    assert_eq!((gauze.stats.repeat_islands, gauze.stats.untied, gauze.stats.ties), (1, 9, 0));
}

fn stamp(tile: &BezPath, a: Vec2, b: Vec2, range: std::ops::RangeInclusive<i32>) -> BezPath {
    let mut out = BezPath::new();
    for k in range.clone() {
        for l in range.clone() {
            let v = a * f64::from(k) + b * f64::from(l);
            out.extend((vectorcraft_geom::kurbo::Affine::translate(v) * tile.clone()).iter());
        }
    }
    crate::ops::simplify(&out).unwrap()
}

#[test]
fn stamping_the_tile_reproduces_the_sheet() {
    let (a, b) = (Vec2::new(20.0, 0.0), Vec2::new(0.0, 20.0));
    let (paints, bounds) = repeated(a, b, 5, 5);
    let p = raw(bounds.width());
    let sheet = compute_repeat(&paints, &p, bounds, Some(lattice(a, b))).unwrap();
    let tiled = compute_repeat(&paints, &Stencil { output: StencilOutput::Tile, ..p.clone() }, bounds, Some(lattice(a, b))).unwrap();
    let tile = tiled.tile.clone().expect("a tile");
    assert_eq!(tile.cell.bounding_box(), Rect::new(40.0, 40.0, 60.0, 60.0));
    assert_eq!(tiled.cut, tile.cut);
    assert_eq!(tiled.sheet, Rect::new(30.0, 30.0, 70.0, 70.0), "the cell plus the margin");
    assert_eq!(tiled.paper, tile.paper);
    assert_eq!((tiled.stats.islands, tiled.stats.repeat_islands), (25, 1), "the sheet's islands stay");
    // Inside the copies away from the sheet's edge, the stamped tiles are the sheet's cut.
    let inner = Rect::new(20.0, 20.0, 90.0, 90.0).to_path(0.01);
    let stamped = crate::ops::boolean_step(&stamp(&tile.cut, a, b, -3..=3), &inner, BoolOp::Intersect).unwrap();
    let want = crate::ops::boolean_step(&sheet.cut, &inner, BoolOp::Intersect).unwrap();
    let diff = crate::ops::boolean_step(&stamped, &want, BoolOp::Exclude).unwrap().area().abs();
    assert!(want.area() > 1000.0 && diff < 1e-6 * want.area().max(1.0) + 1e-6, "differ by {diff} of {}", want.area());
    // Registration holes on the cell's corners (the lattice points).
    let holes = compute_repeat(&paints, &Stencil { output: StencilOutput::Tile, hoshi: true, ..p.clone() }, bounds, Some(lattice(a, b))).unwrap();
    assert_eq!(holes.hoshi, [Point::new(40.0, 40.0), Point::new(60.0, 40.0), Point::new(60.0, 60.0), Point::new(40.0, 60.0)]);
    // A tile without a repeat is the sheet.
    let none = compute_repeat(&paints, &Stencil { output: StencilOutput::Tile, ..p }, bounds, None).unwrap();
    assert!(none.tile.is_none());
    assert_eq!(none.sheet, bounds.inflate(10.0, 10.0));
}
