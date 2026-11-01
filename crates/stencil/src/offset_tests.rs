//! Region offset (grow / shrink) and morphological opening / closing: exact areas on rectangles,
//! circles and rings, containment, holes, degenerate and hostile input, and a seeded sweep over
//! jagged polygons like traced art.

use std::f64::consts::PI;

use crate::ops::StrokeJoin;
use vectorcraft_geom::Rect;
use vectorcraft_geom::kurbo::{BezPath, Circle, PathEl, Point, Shape};

use crate::offset::{MAX_OFFSET_ELEMENTS, close, offset, open};
use crate::ops::PathOpsError;

const JOINS: [StrokeJoin; 3] = [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel];

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    Circle::new((cx, cy), r).to_path(0.01)
}

fn subpaths(p: &BezPath) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = Vec::new();
    for el in p.elements() {
        if let PathEl::MoveTo(_) = el {
            out.push(BezPath::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(*el);
        }
    }
    out
}

fn first_point(p: &BezPath) -> Option<Point> {
    p.elements().iter().find_map(|e| if let PathEl::MoveTo(q) = e { Some(*q) } else { None })
}

/// Even-odd area: a contour nested in an odd number of others is a hole.
fn area(p: &BezPath) -> f64 {
    let subs = subpaths(p);
    let mut total = 0.0;
    for (i, s) in subs.iter().enumerate() {
        let Some(q) = first_point(s) else { continue };
        let depth = subs.iter().enumerate().filter(|(j, o)| *j != i && o.winding(q) != 0).count();
        total += if depth % 2 == 0 { s.area().abs() } else { -s.area().abs() };
    }
    total
}

/// Inside an even-odd result.
fn inside(p: &BezPath, q: Point) -> bool {
    p.winding(q) % 2 != 0
}

/// Inside a non-zero input.
fn inside_nz(p: &BezPath, q: Point) -> bool {
    p.winding(q) != 0
}

fn close_to(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Points of a grid over `r`, offset off the integer lattice so they rarely land on an edge.
fn grid(r: Rect, n: usize) -> Vec<Point> {
    let mut out = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let fx = (i as f64 + 0.371) / n as f64;
            let fy = (j as f64 + 0.613) / n as f64;
            out.push(Point::new(r.x0 + fx * r.width(), r.y0 + fy * r.height()));
        }
    }
    out
}

fn finite(p: &BezPath) -> bool {
    crate::ops::finite(p)
}

// ---------------------------------------------------------------- hostile and degenerate input

#[test]
fn non_finite_input_is_bad_input() {
    let sq = rect(0.0, 0.0, 10.0, 10.0);
    for d in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for j in JOINS {
            assert_eq!(offset(&sq, d, j), Err(PathOpsError::BadInput));
        }
        assert_eq!(open(&sq, d), Err(PathOpsError::BadInput));
        assert_eq!(close(&sq, d), Err(PathOpsError::BadInput));
    }
    for bad in [f64::NAN, f64::INFINITY, 1e12] {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((bad, 0.0));
        p.line_to((0.0, 10.0));
        p.close_path();
        assert_eq!(offset(&p, 1.0, StrokeJoin::Round), Err(PathOpsError::BadInput));
        assert_eq!(offset(&p, -1.0, StrokeJoin::Miter), Err(PathOpsError::BadInput));
        assert_eq!(open(&p, 1.0), Err(PathOpsError::BadInput));
        assert_eq!(close(&p, 1.0), Err(PathOpsError::BadInput));
    }
}

#[test]
fn too_many_elements_is_refused_before_any_work() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    for i in 0..MAX_OFFSET_ELEMENTS {
        p.line_to(((i % 7) as f64, (i % 5) as f64));
    }
    p.close_path();
    assert_eq!(offset(&p, 1.0, StrokeJoin::Round), Err(PathOpsError::BadInput));
}

#[test]
fn degenerate_regions_offset_to_nothing() {
    let mut point = BezPath::new();
    point.move_to((5.0, 5.0));
    let mut point_closed = point.clone();
    point_closed.close_path();
    let zero_area = rect(0.0, 0.0, 10.0, 0.0);
    let mut collinear = BezPath::new();
    collinear.move_to((0.0, 0.0));
    collinear.line_to((5.0, 5.0));
    collinear.line_to((10.0, 10.0));
    collinear.close_path();
    let mut spike = BezPath::new();
    spike.move_to((0.0, 0.0));
    spike.line_to((10.0, 0.0));
    spike.line_to((0.0, 0.0));
    for p in [BezPath::new(), point, point_closed, zero_area, collinear, spike] {
        for d in [-3.0, -0.5, 0.0, 0.5, 3.0] {
            for j in JOINS {
                let r = offset(&p, d, j).unwrap();
                assert!(area(&r).abs() < 1e-6, "{p:?} by {d}: {r:?}");
            }
        }
        assert!(area(&open(&p, 2.0).unwrap()).abs() < 1e-6);
        assert!(area(&close(&p, 2.0).unwrap()).abs() < 1e-6);
    }
}

#[test]
fn zero_distance_returns_the_simplified_region() {
    // Two overlapping squares (non-zero): one region of area 100 + 100 - 25.
    let mut p = rect(0.0, 0.0, 10.0, 10.0);
    p.extend(rect(5.0, 5.0, 15.0, 15.0).elements().iter().copied());
    let r = offset(&p, 0.0, StrokeJoin::Miter).unwrap();
    assert!(close_to(area(&r), 175.0, 1e-6), "{}", area(&r));
    assert_eq!(subpaths(&r).len(), 1);
    assert!(close_to(area(&open(&p, 0.0).unwrap()), 175.0, 1e-6));
    assert!(close_to(area(&close(&p, -4.0).unwrap()), 175.0, 1e-6));
}

#[test]
fn huge_distances_are_clamped_not_crashed() {
    let sq = rect(0.0, 0.0, 10.0, 10.0);
    let g = offset(&sq, 1e300, StrokeJoin::Round).unwrap();
    assert!(finite(&g));
    assert!(inside(&g, Point::new(5.0, 5.0)));
    assert!(offset(&sq, -1e300, StrokeJoin::Round).unwrap().elements().is_empty());
}

// ---------------------------------------------------------------- exact areas

#[test]
fn rectangle_grows_by_join() {
    let (w, h, d) = (40.0, 20.0, 3.0);
    let r = rect(0.0, 0.0, w, h);
    let miter = area(&offset(&r, d, StrokeJoin::Miter).unwrap());
    assert!(close_to(miter, (w + 2.0 * d) * (h + 2.0 * d), 0.01), "{miter}");
    let bevel = area(&offset(&r, d, StrokeJoin::Bevel).unwrap());
    assert!(close_to(bevel, (w + 2.0 * d) * (h + 2.0 * d) - 2.0 * d * d, 0.01), "{bevel}");
    let round = area(&offset(&r, d, StrokeJoin::Round).unwrap());
    // Round corners are chords of the arc (to within 0.5% of the distance).
    assert!(close_to(round, w * h + 2.0 * d * (w + h) + PI * d * d, 0.01 * PI * d * d), "{round}");
}

#[test]
fn rectangle_shrinks_the_same_with_every_join() {
    let (w, h, d) = (40.0, 20.0, 3.0);
    let r = rect(0.0, 0.0, w, h);
    for j in JOINS {
        let s = offset(&r, -d, j).unwrap();
        assert!(close_to(area(&s), (w - 2.0 * d) * (h - 2.0 * d), 0.01), "{j:?}: {}", area(&s));
    }
}

#[test]
fn shrinking_past_half_the_width_leaves_nothing() {
    let r = rect(0.0, 0.0, 100.0, 8.0);
    for j in JOINS {
        for d in [4.01, 5.0, 50.0, 1000.0] {
            let s = offset(&r, -d, j).unwrap();
            assert!(area(&s).abs() < 1e-6, "{j:?} {d}: {}", area(&s));
        }
        // Just under half the width a thin strip survives.
        let s = offset(&r, -3.9, j).unwrap();
        assert!(close_to(area(&s), (100.0 - 7.8) * 0.2, 0.01), "{}", area(&s));
    }
}

#[test]
fn circle_area_follows_the_radius() {
    let (r0, d) = (20.0, 5.0);
    let c = circle(0.0, 0.0, r0);
    let g = area(&offset(&c, d, StrokeJoin::Round).unwrap());
    let expected = PI * (r0 + d) * (r0 + d);
    assert!((g - expected).abs() / expected < 0.005, "{g} vs {expected}");
    let s = area(&offset(&c, -d, StrokeJoin::Round).unwrap());
    let expected = PI * (r0 - d) * (r0 - d);
    assert!((s - expected).abs() / expected < 0.005, "{s} vs {expected}");
    assert!(offset(&c, -r0 - 0.1, StrokeJoin::Round).unwrap().elements().is_empty());
}

// ---------------------------------------------------------------- holes and nesting

fn ring() -> BezPath {
    // 100 × 100 square with a 40 × 40 hole in the middle (hole drawn the other way round, as
    // imported art does; even-odd and non-zero agree).
    let mut p = rect(0.0, 0.0, 100.0, 100.0);
    p.extend(rect(30.0, 30.0, 70.0, 70.0).reverse_subpaths().elements().iter().copied());
    p
}

#[test]
fn holes_shrink_as_the_region_grows_and_grow_as_it_shrinks() {
    let p = ring();
    assert!(close_to(area(&p), 10000.0 - 1600.0, 1e-6));
    let g = offset(&p, 5.0, StrokeJoin::Miter).unwrap();
    assert!(close_to(area(&g), 110.0 * 110.0 - 30.0 * 30.0, 0.01), "{}", area(&g));
    assert!(!inside(&g, Point::new(50.0, 50.0)));
    assert!(inside(&g, Point::new(33.0, 50.0)));
    let s = offset(&p, -5.0, StrokeJoin::Miter).unwrap();
    assert!(close_to(area(&s), 90.0 * 90.0 - 50.0 * 50.0, 0.01), "{}", area(&s));
    assert!(!inside(&s, Point::new(27.0, 50.0)));
    // A hole narrower than twice the distance closes.
    let filled = offset(&p, 21.0, StrokeJoin::Miter).unwrap();
    assert!(inside(&filled, Point::new(50.0, 50.0)));
    assert!(close_to(area(&filled), 142.0 * 142.0, 0.01), "{}", area(&filled));
    assert_eq!(subpaths(&filled).len(), 1);
}

#[test]
fn islands_inside_holes_offset_on_their_own() {
    let mut p = ring();
    p.extend(rect(45.0, 45.0, 55.0, 55.0).elements().iter().copied());
    let g = offset(&p, 2.0, StrokeJoin::Miter).unwrap();
    // Outer 104², hole 36², island 14².
    assert!(close_to(area(&g), 104.0 * 104.0 - 36.0 * 36.0 + 14.0 * 14.0, 0.01), "{}", area(&g));
    assert!(inside(&g, Point::new(50.0, 50.0)));
    assert!(!inside(&g, Point::new(40.0, 50.0)));
    let s = offset(&p, -2.0, StrokeJoin::Miter).unwrap();
    assert!(close_to(area(&s), 96.0 * 96.0 - 44.0 * 44.0 + 6.0 * 6.0, 0.01), "{}", area(&s));
    // The island goes when shrunk past its half-width; the ring stays.
    let s = offset(&p, -6.0, StrokeJoin::Miter).unwrap();
    assert!(!inside(&s, Point::new(50.0, 50.0)));
    assert!(close_to(area(&s), 88.0 * 88.0 - 52.0 * 52.0, 0.01), "{}", area(&s));
}

#[test]
fn input_is_read_non_zero() {
    // Two same-direction overlapping squares fill their overlap under non-zero (even-odd would
    // leave a hole there).
    let mut p = rect(0.0, 0.0, 20.0, 20.0);
    p.extend(rect(10.0, 0.0, 30.0, 20.0).elements().iter().copied());
    let g = offset(&p, 1.0, StrokeJoin::Miter).unwrap();
    assert!(close_to(area(&g), 32.0 * 22.0, 0.01), "{}", area(&g));
    assert!(inside(&g, Point::new(15.0, 10.0)));
}

#[test]
fn self_intersecting_input_offsets_its_filled_area() {
    // A bow tie (two triangles meeting at (10, 10)) and a pentagram (non-zero: solid centre).
    let mut bow = BezPath::new();
    bow.move_to((0.0, 0.0));
    bow.line_to((20.0, 20.0));
    bow.line_to((20.0, 0.0));
    bow.line_to((0.0, 20.0));
    bow.close_path();
    let mut star = BezPath::new();
    for k in 0..5 {
        let a = -PI / 2.0 + k as f64 * 4.0 * PI / 5.0;
        let q = Point::new(100.0 + 50.0 * a.cos(), 100.0 + 50.0 * a.sin());
        if k == 0 {
            star.move_to(q);
        } else {
            star.line_to(q);
        }
    }
    star.close_path();
    for p in [bow, star] {
        let a0 = area(&offset(&p, 0.0, StrokeJoin::Miter).unwrap());
        for j in JOINS {
            let g = offset(&p, 2.0, j).unwrap();
            let s = offset(&p, -2.0, j).unwrap();
            assert!(finite(&g) && finite(&s));
            assert!(area(&g) > a0 && area(&s) < a0 && area(&s) >= 0.0, "{j:?}: {} {a0} {}", area(&g), area(&s));
            let bb = p.bounding_box().inflate(5.0, 5.0);
            for q in grid(bb, 30) {
                if inside_nz(&p, q) {
                    assert!(inside(&g, q), "{q:?} lost by growing");
                }
                if inside(&s, q) {
                    assert!(inside_nz(&p, q), "{q:?} gained by shrinking");
                }
            }
        }
    }
}

// ---------------------------------------------------------------- open / close

#[test]
fn opening_removes_slivers_and_keeps_fat_parts() {
    // A 60 × 60 square with a 4-wide, 40-long bar sticking out of its right side.
    let mut p = rect(0.0, 0.0, 60.0, 60.0);
    p.extend(rect(60.0, 28.0, 100.0, 32.0).elements().iter().copied());
    let r = 3.0;
    let o = open(&p, r).unwrap();
    assert!(!inside(&o, Point::new(80.0, 30.0)), "the sliver survived");
    assert!(inside(&o, Point::new(30.0, 30.0)) && inside(&o, Point::new(1.0, 30.0)) && inside(&o, Point::new(59.0, 30.0)));
    // The square keeps its area less the rounded corners, (4 − π) r² in all, plus a small fillet
    // where the bar joined it (discs of radius r fit a little way into the bar's mouth).
    let expected = 3600.0 - (4.0 - PI) * r * r;
    assert!(area(&o) > expected - 0.5 && area(&o) < expected + 4.0, "{} vs {expected}", area(&o));
    assert!(!inside(&o, Point::new(61.0, 30.0)));
    // A bar wider than 2r is kept.
    let o = open(&p, 1.5).unwrap();
    assert!(inside(&o, Point::new(80.0, 30.0)));
}

#[test]
fn closing_fills_narrow_gaps_and_keeps_wide_ones() {
    // Two 40 × 40 squares 4 apart, and two more 12 apart.
    let mut p = rect(0.0, 0.0, 40.0, 40.0);
    p.extend(rect(44.0, 0.0, 84.0, 40.0).elements().iter().copied());
    let c = close(&p, 3.0).unwrap();
    assert!(inside(&c, Point::new(42.0, 20.0)), "the gap stayed open");
    assert_eq!(subpaths(&c).len(), 1);
    // The outline elsewhere stays put: the bridged block is 84 × 40, less a shallow notch at each
    // end of the gap (a radius-3 disc from outside reaches 3 − √5 ≈ 0.76 into a 4-wide gap).
    assert!(close_to(area(&c), 84.0 * 40.0, 6.0), "{}", area(&c));
    assert!(inside(&c, Point::new(42.0, 1.0)) && inside(&c, Point::new(42.0, 39.0)));
    let mut wide = rect(0.0, 0.0, 40.0, 40.0);
    wide.extend(rect(52.0, 0.0, 92.0, 40.0).elements().iter().copied());
    let c = close(&wide, 3.0).unwrap();
    assert!(!inside(&c, Point::new(46.0, 20.0)));
    assert!(close_to(area(&c), 3200.0, 0.5), "{}", area(&c));
}

#[test]
fn closing_fills_pinholes() {
    let mut p = rect(0.0, 0.0, 50.0, 50.0);
    p.extend(circle(25.0, 25.0, 2.0).reverse_subpaths().elements().iter().copied());
    assert!(!inside(&offset(&p, 0.0, StrokeJoin::Round).unwrap(), Point::new(25.0, 25.0)));
    let c = close(&p, 3.0).unwrap();
    assert!(inside(&c, Point::new(25.0, 25.0)));
    assert!(close_to(area(&c), 2500.0, 0.5), "{}", area(&c));
}

// ---------------------------------------------------------------- seeded jagged sweep

/// SplitMix64: deterministic, identical everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// A jagged star-shaped contour around `c` (like a traced blob): `n` vertices at random radii.
/// With `wild`, vertices are random points in the box instead (self-intersecting).
fn jagged(rng: &mut Rng, c: Point, radius: f64, n: usize, wild: bool) -> BezPath {
    let mut p = BezPath::new();
    for k in 0..n {
        let q = if wild {
            Point::new(c.x + rng.range(-radius, radius), c.y + rng.range(-radius, radius))
        } else {
            let a = k as f64 / n as f64 * 2.0 * PI;
            let r = radius * rng.range(0.55, 1.0);
            Point::new(c.x + r * a.cos(), c.y + r * a.sin())
        };
        if k == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    p.close_path();
    p
}

#[test]
fn jagged_polygon_sweep_keeps_containment_and_order() {
    let mut rng = Rng(0x05EE_D57E_4C11);
    for case in 0..150 {
        let mut p = BezPath::new();
        let contours = 1 + (rng.next() % 4) as usize;
        for _ in 0..contours {
            let c = Point::new(rng.range(0.0, 60.0), rng.range(0.0, 60.0));
            let n = 3 + (rng.next() % 60) as usize;
            let wild = rng.next().is_multiple_of(4);
            let radius = rng.range(5.0, 40.0);
            let mut contour = jagged(&mut rng, c, radius, n, wild);
            if rng.next().is_multiple_of(3) {
                contour = contour.reverse_subpaths();
            }
            p.extend(contour.elements().iter().copied());
        }
        let d = rng.range(0.5, 6.0);
        let join = JOINS[(rng.next() % 3) as usize];
        let a0 = area(&offset(&p, 0.0, join).unwrap());
        let g = offset(&p, d, join).unwrap_or_else(|e| panic!("case {case}: grow {e:?}"));
        let s = offset(&p, -d, join).unwrap_or_else(|e| panic!("case {case}: shrink {e:?}"));
        assert!(finite(&g) && finite(&s), "case {case}");
        assert!(area(&g) + 1e-6 >= a0 && area(&s) <= a0 + 1e-6, "case {case}: {} {a0} {}", area(&g), area(&s));
        let bb = p.bounding_box().inflate(d + 1.0, d + 1.0);
        for q in grid(bb, 24) {
            if inside_nz(&p, q) {
                assert!(inside(&g, q), "case {case}: {q:?} lost by growing");
            }
            if inside(&s, q) {
                assert!(inside_nz(&p, q), "case {case}: {q:?} gained by shrinking");
            }
        }
        let o = open(&p, d).unwrap_or_else(|e| panic!("case {case}: open {e:?}"));
        let c = close(&p, d).unwrap_or_else(|e| panic!("case {case}: close {e:?}"));
        // Opening is anti-extensive and closing extensive (up to the flattening tolerance).
        assert!(area(&o) <= a0 + 0.05 * d * d + 1e-6 && area(&c) + 0.05 * d * d + 1e-6 >= a0, "case {case}: {} {a0} {}", area(&o), area(&c));
    }
}

#[test]
fn many_jagged_contours_offset() {
    let p = traced_like(120, 40);
    for d in [-1.0, 1.0] {
        let r = offset(&p, d, StrokeJoin::Round).unwrap();
        assert!(finite(&r) && !r.elements().is_empty());
    }
}

/// `count` jagged blobs of `n` vertices in a grid, some with a jagged hole, like traced art.
fn traced_like(count: usize, n: usize) -> BezPath {
    let mut rng = Rng(42);
    let mut p = BezPath::new();
    let cols = (count as f64).sqrt().ceil() as usize;
    for i in 0..count {
        let c = Point::new((i % cols) as f64 * 30.0, (i / cols) as f64 * 30.0);
        let radius = rng.range(8.0, 18.0);
        p.extend(jagged(&mut rng, c, radius, n, false).elements().iter().copied());
        if i.is_multiple_of(3) {
            p.extend(jagged(&mut rng, c, 4.0, n / 2, false).reverse_subpaths().elements().iter().copied());
        }
    }
    p
}

/// Timing on ~300 jagged contours (`cargo test -p vectorcraft-stencil --release -- --ignored
/// offset_timing --nocapture`).
#[test]
#[ignore]
fn offset_timing() {
    let p = traced_like(300, 120);
    let elements = p.elements().len();
    for (label, d) in [("shrink 1", -1.0), ("grow 1", 1.0), ("grow 4", 4.0)] {
        for j in JOINS {
            let t = std::time::Instant::now();
            let r = offset(&p, d, j).unwrap();
            eprintln!("{label} {j:?}: {elements} elements in, {} out, {:?}", r.elements().len(), t.elapsed());
        }
    }
    let t = std::time::Instant::now();
    let _ = open(&p, 1.0).unwrap();
    eprintln!("open 1: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let _ = close(&p, 1.0).unwrap();
    eprintln!("close 1: {:?}", t.elapsed());
}
