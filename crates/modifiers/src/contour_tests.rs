//! Tests for the marching-squares core.

use std::f64::consts::PI;

use vectorcraft_geom::Point;

use crate::contour::{CASES, ContourError, Edge, Grid, MAX_CELLS, Ring, bands, contours};

fn total_area(rings: &[Ring]) -> f64 {
    rings.iter().map(Ring::signed_area).sum()
}

/// A 2 × 2 grid (one cell, step 1) with `inside` corners at `hi` and the others at `lo`.
fn one_cell(case: usize, hi: f64, lo: f64) -> Grid {
    let v = |bit: usize| if case & bit != 0 { hi } else { lo };
    // Row-major: TL, TR, BL, BR.
    Grid::new(2, 2, Point::ZERO, 1.0, vec![v(8), v(4), v(1), v(2)]).unwrap()
}

/// Non-zero winding of `rings` at `p` (ray cast to +x).
fn winding(rings: &[Ring], p: Point) -> i32 {
    let mut w = 0;
    for r in rings {
        let n = r.points.len();
        for k in 0..n {
            let a = r.points[k];
            let b = r.points[(k + 1) % n];
            let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y && b.y > p.y && cross > 0.0 {
                w += 1;
            } else if a.y > p.y && b.y <= p.y && cross < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

#[test]
fn case_table_pairs_each_crossing_edge_once() {
    // Corners per edge: T = TL-TR, R = TR-BR, B = BR-BL, L = BL-TL (bits 8, 4, 2, 1).
    let ends = |e: Edge| match e {
        Edge::T => (8, 4),
        Edge::R => (4, 2),
        Edge::B => (2, 1),
        Edge::L => (1, 8),
    };
    for (case, (plain, joined)) in CASES.iter().enumerate() {
        for segs in [*plain, *joined] {
            if segs.is_empty() {
                continue;
            }
            let mut used = Vec::new();
            for &(a, b) in segs {
                used.push(a);
                used.push(b);
            }
            for e in [Edge::T, Edge::R, Edge::B, Edge::L] {
                let (c0, c1) = ends(e);
                let crosses = (case & c0 != 0) != (case & c1 != 0);
                let n = used.iter().filter(|&&u| u == e).count();
                assert_eq!(n, usize::from(crosses), "case {case} edge {e:?}");
            }
        }
        assert_eq!(!joined.is_empty(), case == 5 || case == 10, "only the saddles have a joined form");
    }
}

#[test]
fn every_case_gives_its_area_clockwise() {
    // Inside = 1, outside = 0, level ½: crossings at edge midpoints; corners along the frame.
    let expected = |case: usize| -> (usize, f64) {
        match case.count_ones() {
            0 => (0, 0.0),
            1 => (1, 0.125),
            2 if case == 5 || case == 10 => (1, 0.75), // centre ½ >= ½: joined
            2 => (1, 0.5),
            3 => (1, 0.875),
            _ => (1, 1.0),
        }
    };
    for case in 0..16 {
        let rings = contours(&one_cell(case, 1.0, 0.0), 0.5).unwrap();
        let (n, area) = expected(case);
        assert_eq!(rings.len(), n, "case {case}");
        assert!((total_area(&rings) - area).abs() < 1e-12, "case {case}: {}", total_area(&rings));
        for r in &rings {
            assert!(r.signed_area() > 0.0, "case {case}: outer rings run clockwise");
            assert!(r.points.len() >= 3);
        }
    }
}

#[test]
fn saddles_join_when_the_centre_is_inside() {
    for case in [5, 10] {
        let rings = contours(&one_cell(case, 1.0, 0.0), 0.5).unwrap();
        assert_eq!(rings.len(), 1, "case {case} joined");
        assert!((total_area(&rings) - 0.75).abs() < 1e-12);
    }
}

#[test]
fn saddles_split_when_the_centre_is_outside() {
    // Outside corners at -½: centre mean ¼ < ½. Crossings a third of the way from each inside corner.
    for case in [5, 10] {
        let rings = contours(&one_cell(case, 1.0, -0.5), 0.5).unwrap();
        assert_eq!(rings.len(), 2, "case {case} split");
        for r in &rings {
            assert!((r.signed_area() - 1.0 / 18.0).abs() < 1e-12, "{}", r.signed_area());
        }
    }
}

#[test]
fn saddle_centre_exactly_on_the_level_joins() {
    let g = Grid::new(2, 2, Point::ZERO, 1.0, vec![2.0, -1.0, -1.0, 2.0]).unwrap();
    // mean ½ == level ½ → joined.
    assert_eq!(contours(&g, 0.5).unwrap().len(), 1);
}

fn disc_grid(r: f64, step: f64) -> Grid {
    let n = (100.0 / step) as usize + 1;
    Grid::sample(n, n, Point::ZERO, step, |p| r - (p - Point::new(50.0, 50.0)).hypot()).unwrap()
}

#[test]
fn circle_area_within_one_percent() {
    let r = 40.0;
    let rings = contours(&disc_grid(r, 0.25), 0.0).unwrap();
    assert_eq!(rings.len(), 1);
    let a = total_area(&rings);
    let want = PI * r * r;
    assert!(((a - want) / want).abs() < 0.01, "{a} vs {want}");
    // Points lie on the circle.
    for p in &rings[0].points {
        assert!(((*p - Point::new(50.0, 50.0)).hypot() - r).abs() < 0.01);
    }
}

#[test]
fn holes_run_opposite_to_outers() {
    // A ring 20 <= d <= 40 round (50, 50), plus a disc d < 10 inside its hole.
    let c = Point::new(50.0, 50.0);
    let g = Grid::sample(201, 201, Point::ZERO, 0.5, |p| {
        let d = (p - c).hypot();
        (d - 20.0).min(40.0 - d).max(10.0 - d)
    })
    .unwrap();
    let rings = contours(&g, 0.0).unwrap();
    assert_eq!(rings.len(), 3);
    let mut areas: Vec<f64> = rings.iter().map(Ring::signed_area).collect();
    areas.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
    assert!(areas[0] > 0.0, "the inner disc is an outer");
    assert!(areas[1] < 0.0, "the annulus's inner edge is a hole");
    assert!(areas[2] > 0.0, "the annulus's outer edge is an outer");
    let want = PI * (40.0f64.powi(2) - 20.0f64.powi(2) + 10.0f64.powi(2));
    assert!(((total_area(&rings) - want) / want).abs() < 0.01);
    // Non-zero and even-odd agree: winding is 0 or 1 everywhere.
    for (x, inside) in [(50.0, true), (65.0, false), (80.0, true), (95.0, false), (2.0, false)] {
        let w = winding(&rings, Point::new(x, 50.3));
        assert_eq!(w, i32::from(inside), "at x {x}");
    }
}

#[test]
fn bands_match_the_min_trick_and_make_a_hole() {
    let c = Point::new(50.0, 50.0);
    let g = Grid::sample(201, 201, Point::ZERO, 0.5, |p| (p - c).hypot()).unwrap();
    let rings = bands(&g, 20.0, 40.0).unwrap();
    assert_eq!(rings.len(), 2);
    assert_eq!(rings.iter().filter(|r| r.is_hole()).count(), 1);
    let want = PI * (40.0f64.powi(2) - 20.0f64.powi(2));
    assert!(((total_area(&rings) - want) / want).abs() < 0.01);
    assert_eq!(bands(&g, 5.0, 1.0), Err(ContourError::BadLevel));
    assert_eq!(bands(&g, f64::NAN, 1.0), Err(ContourError::BadLevel));
}

#[test]
fn empty_and_full_grids() {
    let out = Grid::new(3, 3, Point::ZERO, 2.0, vec![0.0; 9]).unwrap();
    assert!(contours(&out, 1.0).unwrap().is_empty());
    let full = Grid::new(4, 3, Point::new(10.0, 20.0), 2.0, vec![5.0; 12]).unwrap();
    let rings = contours(&full, 1.0).unwrap();
    assert_eq!(rings.len(), 1);
    // The grid's rectangle, its edge samples dropped as collinear.
    assert_eq!(rings[0].points.len(), 4);
    assert!((rings[0].signed_area() - 6.0 * 4.0).abs() < 1e-9);
    for p in &rings[0].points {
        assert!((p.x == 10.0 || p.x == 16.0) && (p.y == 20.0 || p.y == 24.0));
    }
}

#[test]
fn level_equal_to_values_counts_as_inside() {
    let g = Grid::new(2, 2, Point::ZERO, 1.0, vec![1.0; 4]).unwrap();
    assert_eq!(contours(&g, 1.0).unwrap().len(), 1);
}

#[test]
fn non_finite_samples_are_outside() {
    // A full grid with one NaN in the middle: a hole round it.
    let mut v = vec![1.0; 25];
    v[12] = f64::NAN;
    let g = Grid::new(5, 5, Point::ZERO, 1.0, v.clone()).unwrap();
    let rings = contours(&g, 0.5).unwrap();
    assert_eq!(rings.len(), 2);
    assert_eq!(rings.iter().filter(|r| r.is_hole()).count(), 1);
    for bad in [f64::INFINITY, f64::NEG_INFINITY] {
        v[12] = bad;
        let g = Grid::new(5, 5, Point::ZERO, 1.0, v.clone()).unwrap();
        assert_eq!(contours(&g, 0.5).unwrap().len(), 2, "{bad} is outside");
    }
    let nan = Grid::new(3, 3, Point::ZERO, 1.0, vec![f64::NAN; 9]).unwrap();
    assert!(contours(&nan, 0.0).unwrap().is_empty());
}

#[test]
fn junk_sizes_and_params_are_refused() {
    assert_eq!(Grid::new(0, 0, Point::ZERO, 1.0, vec![]), Err(ContourError::TooSmall { cols: 0, rows: 0 }));
    assert_eq!(Grid::new(1, 5, Point::ZERO, 1.0, vec![0.0; 5]), Err(ContourError::TooSmall { cols: 1, rows: 5 }));
    assert!(matches!(Grid::new(usize::MAX, usize::MAX, Point::ZERO, 1.0, vec![]), Err(ContourError::TooLarge { .. })));
    assert!(matches!(Grid::new(1002, 1002, Point::ZERO, 1.0, vec![]), Err(ContourError::TooLarge { .. })));
    assert!(matches!(Grid::sample(5000, 5000, Point::ZERO, 1.0, |_| 0.0), Err(ContourError::TooLarge { .. })));
    assert_eq!(Grid::new(2, 2, Point::ZERO, 1.0, vec![0.0; 3]), Err(ContourError::WrongLength { expected: 4, got: 3 }));
    for s in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(Grid::new(2, 2, Point::ZERO, s, vec![0.0; 4]), Err(ContourError::BadStep));
    }
    assert_eq!(Grid::new(2, 2, Point::new(f64::NAN, 0.0), 1.0, vec![0.0; 4]), Err(ContourError::BadOrigin));
    let g = Grid::new(2, 2, Point::ZERO, 1.0, vec![0.0; 4]).unwrap();
    for l in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(contours(&g, l), Err(ContourError::BadLevel));
    }
    // Every error says something.
    assert!(!ContourError::TooLarge { cells: 9 }.to_string().is_empty());
}

#[test]
fn the_largest_grid_fits() {
    let g = Grid::sample(1001, 1001, Point::ZERO, 1.0, |p| 300.0 - (p - Point::new(500.0, 500.0)).hypot()).unwrap();
    let rings = contours(&g, 0.0).unwrap();
    assert_eq!(rings.len(), 1);
    assert_eq!(MAX_CELLS, 1000 * 1000);
}

#[test]
fn contouring_is_deterministic() {
    let g = Grid::sample(80, 60, Point::new(-3.0, 7.0), 0.7, |p| (p.x * 0.3).sin() + (p.y * 0.21).cos()).unwrap();
    let a = contours(&g, 0.2).unwrap();
    let b = contours(&g, 0.2).unwrap();
    assert_eq!(a, b);
    assert!(a.len() > 4);
}

/// A small deterministic generator for the junk sweep.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 31) as f64
    }
}

#[test]
fn junk_sweep_gives_closed_finite_rings_in_the_grid() {
    let mut rng = Lcg(0x5eed);
    for round in 0..400 {
        let cols = 2 + (rng.next() % 24) as usize;
        let rows = 2 + (rng.next() % 24) as usize;
        let step = 0.1 + rng.unit() * 3.0;
        let origin = Point::new(rng.unit() * 100.0 - 50.0, rng.unit() * 100.0 - 50.0);
        let values: Vec<f64> = (0..cols * rows)
            .map(|_| match rng.next() % 12 {
                0 => f64::NAN,
                1 => f64::INFINITY,
                2 => f64::NEG_INFINITY,
                3 => 1e300,
                4 => -1e300,
                5 => 0.5, // exactly the level
                _ => rng.unit(),
            })
            .collect();
        let g = Grid::new(cols, rows, origin, step, values.clone()).unwrap();
        let rings = contours(&g, 0.5).unwrap();
        let (x1, y1) = (origin.x + (cols - 1) as f64 * step, origin.y + (rows - 1) as f64 * step);
        for r in &rings {
            assert!(r.points.len() >= 3, "round {round}");
            assert!(r.signed_area().abs() > 0.0);
            for (k, p) in r.points.iter().enumerate() {
                assert!(p.x.is_finite() && p.y.is_finite());
                assert!(p.x >= origin.x - 1e-9 && p.x <= x1 + 1e-9 && p.y >= origin.y - 1e-9 && p.y <= y1 + 1e-9, "round {round}");
                assert_ne!(*p, r.points[(k + 1) % r.points.len()], "no repeated points");
            }
        }
        // A cell whose four corners are well inside makes some area. (A lone inside sample next to
        // non-finite ones collapses to a point and is dropped.)
        let well_in = |i: usize, j: usize| values[j * cols + i].is_finite() && values[j * cols + i] > 0.5;
        let full_cell =
            (0..rows - 1).any(|j| (0..cols - 1).any(|i| well_in(i, j) && well_in(i + 1, j) && well_in(i, j + 1) && well_in(i + 1, j + 1)));
        assert!(!full_cell || !rings.is_empty(), "round {round}");
        for _ in 0..4 {
            let _ = bands(&g, rng.unit() * 0.5, 0.5 + rng.unit() * 0.5).unwrap();
        }
        assert_eq!(rings, contours(&g, 0.5).unwrap());
    }
}

#[test]
fn winding_is_zero_or_one_on_random_smooth_fields() {
    let mut rng = Lcg(42);
    for _ in 0..20 {
        let (fx, fy, ph) = (rng.unit() * 0.5, rng.unit() * 0.5, rng.unit() * 6.0);
        let g = Grid::sample(60, 60, Point::ZERO, 1.0, |p| (p.x * fx + ph).sin() * (p.y * fy).cos()).unwrap();
        let rings = contours(&g, 0.1).unwrap();
        for _ in 0..50 {
            let p = Point::new(0.5 + rng.unit() * 58.0, 0.5 + rng.unit() * 58.0);
            let w = winding(&rings, p);
            assert!(w == 0 || w == 1, "winding {w}");
        }
    }
}

/// Timing for the perf note (`cargo test --release -p vectorcraft-modifiers -- --ignored perf`).
#[test]
#[ignore]
fn perf_one_million_cells() {
    let t = std::time::Instant::now();
    let g = Grid::sample(1001, 1001, Point::ZERO, 0.1, |p| (p.x * 0.7).sin() + (p.y * 0.9).cos()).unwrap();
    let sampled = t.elapsed();
    let rings = contours(&g, 0.3).unwrap();
    eprintln!("1000×1000 cells: sample {sampled:?}, contour {:?}, {} rings", t.elapsed() - sampled, rings.len());
}
