// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, Vec2, shapes};

use super::displace::{Field, Frame, fbm, perm_table, ramp, simplex, sin_cos};
use super::{GeometryOp, GeometryRow, RowAt, Shaper, shape_path};
use crate::model::{Displace, DisplaceMode, ModifierKind};
use crate::{Output, evaluate, parse};

fn stack(rows: Value) -> crate::Stack {
    parse(&json!({"v": 1, "stack": rows}).to_string()).unwrap()
}

fn path_node(sp: SubPath) -> Node {
    Node::path(NodeId(1), PathData::new(vec![sp]), Appearance::default_art())
}

fn square() -> SubPath {
    shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).subpaths[0].clone()
}

fn circle() -> SubPath {
    shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).subpaths[0].clone()
}

fn spatial() -> Displace {
    Displace { progressive: false, ..Displace::default() }
}

fn radial() -> Displace {
    Displace { mode: DisplaceMode::Radial, progressive: false, ..Displace::default() }
}

fn bits(sp: &SubPath) -> Vec<[u64; 6]> {
    sp.anchors.iter().map(|a| [a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y].map(f64::to_bits)).collect()
}

#[test]
fn defaults_and_ranges_are_tlproceduruals() {
    let ModifierKind::Displace(d) = crate::Modifier::new("m", "displace").unwrap().kind else { panic!() };
    // NOISE_DEFAULTS.
    assert_eq!(d.mode, DisplaceMode::Spatial);
    assert_eq!((d.amplitude, d.scale, d.octaves, d.seed, d.ripple_count), (10.0, 0.05, 3, 42, 4));
    assert!(d.progressive && !d.progressive_reverse);
    assert_eq!(d.progressive_amount, 1.0);
    let params = ModifierKind::Displace(d).params();
    for key in ["mode", "amplitude", "scale", "octaves", "rippleCount", "seed", "progressive", "progressiveAmount", "progressiveReverse"] {
        assert!(params.contains_key(key), "{key} in {params:?}");
    }
    assert_eq!(params["mode"], "spatial");
    // NOISE_SCHEMA's ranges, junk clamped with a warning each.
    let out = evaluate(
        &stack(
            json!([{"id": "d", "kind": "displace", "amplitude": 1e9, "scale": 0, "octaves": 99, "rippleCount": 1, "seed": 5e9, "progressiveAmount": -2}]),
        ),
        Rect::new(0.0, 0.0, 100.0, 100.0),
    );
    let GeometryOp::Displace(p) = &out.geometry[0].op else { panic!() };
    assert_eq!((p.amplitude, p.scale, p.octaves, p.ripple_count, p.seed, p.progressive_amount), (100.0, 0.01, 6, 2, 999_999, 0.0));
    assert!(out.warnings.len() >= 6, "{:?}", out.warnings);
    // A mode that isn't one doesn't read: the row is kept but ignored.
    let out = evaluate(&stack(json!([{"id": "d", "kind": "displace", "mode": "wobble"}])), Rect::new(0.0, 0.0, 1.0, 1.0));
    assert!(!out.has_geometry());
}

#[test]
fn the_permutation_table_is_a_seeded_permutation_repeated() {
    for seed in [0, 1, 42, 999_999] {
        let p = perm_table(seed);
        assert_eq!(p.len(), 512);
        let mut first: Vec<u8> = p[..256].to_vec();
        assert_eq!(&p[..256], &p[256..]);
        first.sort_unstable();
        assert_eq!(first, (0..=255u8).collect::<Vec<_>>(), "seed {seed}");
    }
    assert_ne!(perm_table(1), perm_table(2));
    assert_eq!(perm_table(7), perm_table(7));
}

#[test]
fn simplex_and_its_fractal_sum_stay_in_minus_one_to_one() {
    let p = perm_table(42);
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for i in -60..60 {
        for j in -60..60 {
            let (x, y) = (f64::from(i) * 0.173, f64::from(j) * 0.191);
            let n = simplex(x, y, &p);
            lo = lo.min(n);
            hi = hi.max(n);
            for octaves in 1..=6 {
                let f = fbm(x, y, octaves, &p);
                assert!((-1.0..=1.0).contains(&f), "{f}");
            }
        }
    }
    assert!(lo >= -1.0 && hi <= 1.0, "{lo}..{hi}");
    assert!(lo < -0.5 && hi > 0.5, "the noise varies: {lo}..{hi}");
    // Integer lattice points of simplex noise are zero (each corner's gradient term vanishes).
    assert_eq!(simplex(0.0, 0.0, &p), 0.0);
    // Junk coordinates read as no noise.
    assert_eq!(simplex(f64::NAN, 1.0, &p), 0.0);
    assert_eq!(simplex(f64::INFINITY, 1.0, &p), 0.0);
    assert!(fbm(1e300, -1e300, 6, &p).is_finite());
}

#[test]
fn sin_cos_is_the_platforms_to_the_last_few_bits() {
    for i in 0..=2000 {
        let x = f64::from(i) / 2000.0 * std::f64::consts::TAU;
        let (s, c) = sin_cos(x);
        assert!((s - x.sin()).abs() < 1e-14 && (c - x.cos()).abs() < 1e-14, "{x}: {s} {c}");
    }
    assert_eq!(sin_cos(f64::NAN), (0.0, 1.0));
}

#[test]
fn spatial_moves_each_anchor_by_noise_where_it_sits_with_its_handles() {
    let c = circle();
    let f = Field::new(&spatial(), Point::ZERO, None, Frame::IDENTITY);
    let d = f.displace(&c).unwrap();
    assert_eq!(d.anchors.len(), c.anchors.len());
    assert!(d.closed);
    let p = perm_table(42);
    for (a, b) in c.anchors.iter().zip(&d.anchors) {
        let off = b.p - a.p;
        // tlprocedural: (fbm(x·scale, y·scale), fbm(x·scale + 100, y·scale + 100)) × amplitude.
        let want_x = fbm(a.p.x * 0.05, a.p.y * 0.05, 3, &p) * 10.0;
        let want_y = fbm(a.p.x * 0.05 + 100.0, a.p.y * 0.05 + 100.0, 3, &p) * 10.0;
        assert!((off.x - want_x).abs() < 1e-9 && (off.y - want_y).abs() < 1e-9, "{off:?} vs {want_x} {want_y}");
        assert!(off.hypot() <= 10.0 * std::f64::consts::SQRT_2 + 1e-9);
        // The handles move with their anchor: the curve's shape round it is kept.
        assert!(((b.h_in - b.p) - (a.h_in - a.p)).hypot() < 1e-9);
        assert!(((b.h_out - b.p) - (a.h_out - a.p)).hypot() < 1e-9);
    }
    assert!(c.anchors.iter().zip(&d.anchors).any(|(a, b)| a.p.distance(b.p) > 0.1), "something moved");
}

#[test]
fn radial_ripples_along_each_anchors_direction_from_the_centre() {
    // 24 points round (0, 0) at radius 100, as a polygon.
    let pts: Vec<Point> =
        (0..24).map(|i| f64::from(i) / 24.0 * std::f64::consts::TAU).map(|t| Point::new(100.0 * t.cos(), 100.0 * t.sin())).collect();
    let sp = SubPath::polyline(&pts, true);
    let p = Displace { amplitude: 40.0, ripple_count: 4, ..radial() };
    let d = Field::new(&p, Point::ZERO, None, Frame::IDENTITY).displace(&sp).unwrap();
    let mut radii = vec![];
    for (a, b) in sp.anchors.iter().zip(&d.anchors) {
        // Along the radius: the cross product of where it was and where it moved is zero.
        let (u, off) = (a.p.to_vec2(), b.p - a.p);
        assert!(u.cross(off).abs() < 1e-6, "radial: {u:?} {off:?}");
        // At most a quarter of amplitude% of the distance.
        assert!(off.hypot() <= 100.0 * 0.4 * 0.25 + 1e-9);
        radii.push(b.p.to_vec2().hypot());
    }
    // Four ripples round: every sixth point (a quarter turn) is the same distance out.
    for i in 0..18 {
        assert!((radii[i] - radii[i + 6]).abs() < 1e-9, "{radii:?}");
    }
    assert!(radii.iter().any(|r| *r > 101.0) && radii.iter().any(|r| *r < 99.0), "{radii:?}");
    // tlprocedural's formula, with the copy's seeded phase.
    let phase = super::displace::phase(42, 0);
    let (a, b) = (sp.anchors[1].p, d.anchors[1].p);
    let theta = (a.y).atan2(a.x);
    let want = 100.0 * 0.4 * 0.25 * (theta * 4.0 + phase).sin();
    assert!((b.to_vec2().hypot() - 100.0 - want).abs() < 1e-9, "{} vs {want}", b.to_vec2().hypot() - 100.0);
    // A point at the centre doesn't move.
    let at_centre = SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0)], false);
    let moved = Field::new(&p, Point::ZERO, None, Frame::IDENTITY).displace(&at_centre).unwrap();
    assert_eq!(moved.anchors[0].p, Point::ZERO);
}

/// Each copy gets its own phase: spread round the turn, not alternating between two (as
/// tlprocedural's LCG of `seed + index` did: its step is a fixed 0.514 of a turn).
#[test]
fn radial_phases_differ_copy_by_copy() {
    use std::f64::consts::TAU;
    let phases: Vec<f64> = (0..8).map(|k| super::displace::phase(42, k)).collect();
    assert!(phases.iter().all(|p| (0.0..=TAU).contains(p)));
    assert_eq!(phases, (0..8).map(|k| super::displace::phase(42, k)).collect::<Vec<_>>(), "seeded");
    // The old LCG put copies 0 and 2 within 0.03 of a turn of each other.
    let lcg = |k: u64| (((42 + k) * 1_103_515_245 + 12_345) & 0x7fff_ffff) as f64 / f64::from(0x7fff_ffff);
    assert!(((lcg(2) - lcg(0)).abs() - 0.028).abs() < 0.01);
    let mut gaps = 0;
    for i in 0..8 {
        for j in i + 1..8 {
            let d = (phases[i] - phases[j]).abs() / TAU;
            gaps += usize::from(d.min(1.0 - d) < 0.03);
        }
    }
    assert!(gaps <= 2, "{phases:?}");
}

#[test]
fn the_same_seed_gives_the_same_bits_and_another_seed_another_shape() {
    let c = circle();
    for p in [spatial(), radial()] {
        let one = Field::new(&p, Point::new(100.0, 100.0), None, Frame::IDENTITY).displace(&c).unwrap();
        let two = Field::new(&p, Point::new(100.0, 100.0), None, Frame::IDENTITY).displace(&c).unwrap();
        assert_eq!(bits(&one), bits(&two));
        let other = Field::new(&Displace { seed: 7, ..p.clone() }, Point::new(100.0, 100.0), None, Frame::IDENTITY).displace(&c).unwrap();
        assert_ne!(bits(&one), bits(&other), "{:?}", p.mode);
    }
    // A pinned value, so a change to the maths (or a platform that computes it differently) shows.
    // The bits are literals: worked out once on this build, so any platform that computes them
    // differently (or any change to the maths) fails here.
    let d = shape_path(&PathData::new(vec![square()]), &GeometryOp::Displace(spatial()));
    let a = d.subpaths[0].anchors[2].p;
    assert_eq!((a.x.to_bits(), a.y.to_bits()), (SPATIAL_BITS.0, SPATIAL_BITS.1), "spatial {a:?}");
    let r = Field::new(&Displace { amplitude: 60.0, ripple_count: 3, ..radial() }, Point::new(50.0, 50.0), Some((2, 4)), Frame::IDENTITY)
        .displace(&square())
        .unwrap();
    let b = r.anchors[1].p;
    assert_eq!((b.x.to_bits(), b.y.to_bits()), (RADIAL_BITS.0, RADIAL_BITS.1), "radial {b:?}");
}

/// [`the_same_seed_gives_the_same_bits_and_another_seed_another_shape`]'s pinned anchors.
const SPATIAL_BITS: (u64, u64) = (4636742559579634236, 4637038045916675944);
const RADIAL_BITS: (u64, u64) = (4636237433407583925, 4619683745580225710);

#[test]
fn zero_amplitude_and_degenerate_paths_are_left_alone() {
    let zero = Displace { amplitude: 0.0, ..spatial() };
    assert!(Field::new(&zero, Point::ZERO, None, Frame::IDENTITY).displace(&square()).is_none());
    let f = Field::new(&spatial(), Point::ZERO, None, Frame::IDENTITY);
    assert!(f.displace(&SubPath::new(vec![], true)).is_none());
    // One point, or all on one point: moved like any anchor (radial: not at all, at the centre).
    let one = SubPath::polyline(&[Point::new(3.0, 4.0)], false);
    assert_eq!(f.displace(&one).unwrap().anchors.len(), 1);
    let same = SubPath::polyline(&[Point::new(3.0, 4.0); 3], true);
    let r = Field::new(&radial(), Point::new(3.0, 4.0), None, Frame::IDENTITY).displace(&same).unwrap();
    assert!(r.anchors.iter().all(|a| a.p == Point::new(3.0, 4.0)));
}

#[test]
fn junk_geometry_never_panics_and_stays_finite() {
    let weird = [
        SubPath::new(vec![], false),
        SubPath::polyline(&[Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::ZERO, Point::ZERO], true),
        SubPath::polyline(&[Point::new(1e300, -1e300), Point::new(-1e300, 1e300)], false),
        SubPath::polyline(&[Point::new(f64::MAX, f64::MAX), Point::new(-f64::MAX, f64::MAX)], true),
        SubPath::new(vec![Anchor::with_handles(Point::ZERO, Point::new(5.0, 5.0), Point::new(-5.0, 5.0)), Anchor::corner(Point::ZERO)], true),
        SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)], false),
    ];
    let ops = [
        Displace { amplitude: 100.0, scale: 1.0, octaves: 6, ..spatial() },
        Displace { amplitude: 100.0, ripple_count: 12, seed: 999_999, ..radial() },
        // Progressive: copy 0 gets none of it, copies 1 and 2 some.
        Displace::default(),
        // Unclamped junk, as a public `GeometryOp` or `shape_path` may pass: the loops are capped
        // inside the field, and what isn't finite is refused by the shaper.
        Displace { amplitude: f64::NAN, octaves: u32::MAX, ..spatial() },
        Displace { amplitude: 1e308, scale: f64::INFINITY, octaves: u32::MAX, ..spatial() },
        Displace { amplitude: 1e308, ripple_count: u32::MAX, ..radial() },
    ];
    let mut moved = 0;
    for sp in &weird {
        for p in &ops {
            let rows = vec![GeometryRow { modifier: "m".into(), op: GeometryOp::Displace(p.clone()), at: RowAt::Copies }];
            let out = Output { instances: vec![crate::Instance::identity(); 3], geometry: rows, ..Output::default() };
            let mut sh = Shaper::new(&out);
            for k in 0..3 {
                let before = path_node(sp.clone());
                let mut n = before.clone();
                sh.copy_at(&mut n, k);
                let ok = n.path_data().unwrap().subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite());
                let was_ok = sp.anchors.iter().all(|a| a.p.is_finite());
                assert!(ok || !was_ok, "{p:?} on {sp:?}");
                moved += usize::from(n != before);
            }
            // Straight to the field and to `shape_path`, without the shaper's checks: no panic.
            let _ = Field::new(p, Point::new(1.0, 1.0), Some((7, 3)), Frame::IDENTITY).displace(sp);
            let _ = shape_path(&PathData::new(vec![sp.clone()]), &GeometryOp::Displace(p.clone()));
        }
    }
    // The rows did run: most finite paths moved under the finite params.
    assert!(moved >= 20, "only {moved} moved");
}

#[test]
fn progressive_ramps_the_amount_across_the_copies() {
    let p = Displace::default();
    assert_eq!(ramp(&p, None), 1.0, "on the object: all of it");
    assert_eq!(ramp(&p, Some((0, 1))), 1.0, "one copy: all of it");
    let r: Vec<f64> = (0..5).map(|k| ramp(&p, Some((k, 5)))).collect();
    assert_eq!(r, [0.0, 0.25, 0.5, 0.75, 1.0]);
    let rev = Displace { progressive_reverse: true, progressive_amount: 0.5, ..p.clone() };
    let r: Vec<f64> = (0..3).map(|k| ramp(&rev, Some((k, 3)))).collect();
    assert_eq!(r, [0.5, 0.25, 0.0]);
    let off = Displace { progressive: false, ..p };
    assert_eq!(ramp(&off, Some((0, 9))), 1.0);
    // An index past the end reads as the last copy.
    assert_eq!(ramp(&Displace::default(), Some((99, 3))), 1.0);
}

/// Below an array, each copy is displaced where it sits (spatial) or with its own phase (radial):
/// copies differ. Above it, the object is displaced once and every copy is the same shape.
#[test]
fn copies_below_an_array_differ_and_above_it_match() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    for mode in ["spatial", "radial"] {
        let shapes_of = |rows: Value| -> Vec<Vec<Point>> {
            let out = evaluate(&stack(rows), b);
            let (copies, notes) = super::shaped_copies(&path_node(square()), &out);
            assert!(notes.is_empty(), "{notes:?}");
            // Each copy moved back to the first copy's place.
            copies
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    c.path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p - vectorcraft_geom::Vec2::new(150.0 * k as f64, 0.0)).collect()
                })
                .collect()
        };
        let below = shapes_of(json!([
            {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
            {"id": "d", "kind": "displace", "mode": mode, "progressive": false},
        ]));
        assert_eq!(below.len(), 3);
        assert_ne!(below[0], below[1], "{mode}");
        assert_ne!(below[1], below[2], "{mode}");
        let above = shapes_of(json!([
            {"id": "d", "kind": "displace", "mode": mode},
            {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
        ]));
        for c in &above[1..] {
            for (p, q) in c.iter().zip(&above[0]) {
                assert!(p.distance(*q) < 1e-9, "{mode}");
            }
        }
        assert_ne!(above[0], square().anchors.iter().map(|a| a.p).collect::<Vec<_>>(), "{mode}: displaced");
        // Progressive (the default): the first copy below the array isn't moved, the last fully.
        let ramped = shapes_of(json!([
            {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
            {"id": "d", "kind": "displace", "mode": mode},
        ]));
        assert_eq!(ramped[0], square().anchors.iter().map(|a| a.p).collect::<Vec<_>>(), "{mode}");
        assert_eq!(ramped[2], below[2], "{mode}");
    }
}

/// A lopsided 14-point outline in 0..100 (no mirror symmetry of its own).
fn lopsided() -> SubPath {
    let pts: Vec<Point> = (0..14)
        .map(|i| {
            let t = f64::from(i) / 14.0 * std::f64::consts::TAU;
            let r = 30.0 + 15.0 * f64::from(i % 3) + f64::from(i);
            Point::new(50.0 + r * t.cos(), 50.0 + r * t.sin())
        })
        .collect();
    SubPath::polyline(&pts, true)
}

/// `rows`' copies of [`lopsided`], with their instances.
fn copies_of(rows: Value) -> (Output, Vec<Vec<Point>>) {
    let out = evaluate(&stack(rows), Rect::new(0.0, 0.0, 100.0, 100.0));
    let (copies, notes) = super::shaped_copies(&path_node(lopsided()), &out);
    assert!(notes.is_empty(), "{notes:?}");
    let pts = copies.iter().map(|c| c.path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect()).collect();
    (out, pts)
}

/// How far `a`'s points are from `b`'s, one by one, at most.
fn apart(a: &[Point], b: &[Point]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(p, q)| p.distance(*q)).fold(0.0, f64::max)
}

/// Each mirrored copy against its partner (the copy at the same array place, not reflected): the
/// reflection that takes the partner's placement to the mirrored copy's, applied to the partner.
fn mirror_pairs(out: &Output, pts: &[Vec<Point>]) -> Vec<(usize, usize, f64)> {
    let det = |t: vectorcraft_geom::Affine| {
        let [a, b, c, d, ..] = t.as_coeffs();
        a * d - b * c
    };
    let mut pairs = vec![];
    for (i, a) in out.instances.iter().enumerate() {
        if det(a.transform) > 0.0 {
            for (j, b) in out.instances.iter().enumerate() {
                if det(b.transform) < 0.0 && out.ramp_position(a) == out.ramp_position(b) {
                    let m = b.transform * a.transform.inverse();
                    let mapped: Vec<Point> = pts[i].iter().map(|p| m * *p).collect();
                    pairs.push((i, j, apart(&mapped, &pts[j])));
                }
            }
        }
    }
    pairs
}

/// A lone mirror: both halves get the whole effect (no array to ramp over), and in radial mode
/// the mirrored half is the mirror image of the other (same phase, read in its reflected frame).
#[test]
fn a_lone_mirror_displaces_both_halves_fully_and_radial_stays_symmetric() {
    let plain = lopsided().anchors.iter().map(|a| a.p).collect::<Vec<_>>();
    for mode in ["spatial", "radial"] {
        let (out, pts) = copies_of(json!([{"id": "m", "kind": "mirror"}, {"id": "d", "kind": "displace", "mode": mode, "amplitude": 40}]));
        assert_eq!(pts.len(), 2);
        assert!(out.instances.iter().all(|i| out.ramp_position(i).is_none()));
        // The unreflected half moved (the ramp would have left a first copy alone).
        let first = out.instances.iter().position(|i| i.transform == vectorcraft_geom::Affine::IDENTITY).unwrap();
        assert!(apart(&pts[first], &plain) > 1.0, "{mode}: moved");
        let pairs = mirror_pairs(&out, &pts);
        assert_eq!(pairs.len(), 1, "{mode}");
        if mode == "radial" {
            assert!(pairs[0].2 < 1e-9, "radial: a mirror image, {pairs:?}");
        } else {
            // Spatial noise is read where each copy lies: the halves differ.
            assert!(pairs[0].2 > 0.1, "spatial: {pairs:?}");
        }
    }
}

/// An array then a mirror: the ramp steps over the array copies only, each mirror pair sharing a
/// place (the same amount and, radial, the same phase: symmetric pairs); the first pair is left
/// alone.
#[test]
fn an_array_then_a_mirror_ramps_the_pairs_together() {
    let (out, pts) = copies_of(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
        {"id": "m", "kind": "mirror", "offset": 400},
        {"id": "d", "kind": "displace", "mode": "radial", "amplitude": 60, "rippleCount": 3},
    ]));
    assert_eq!(pts.len(), 6);
    let pairs = mirror_pairs(&out, &pts);
    assert_eq!(pairs.len(), 3, "{pairs:?}");
    for (i, j, gap) in &pairs {
        assert!(*gap < 1e-9, "pair {i}/{j} symmetric: {gap}");
    }
    // Place 0 (both halves) unmoved, the others moved.
    for (k, inst) in out.instances.iter().enumerate() {
        let (place, total) = out.ramp_position(inst).unwrap();
        assert_eq!(total, 3);
        let mut unmoved = path_node(lopsided());
        unmoved.transform(inst.transform, true);
        let base: Vec<Point> = unmoved.path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(apart(&pts[k], &base) < 1e-9, place == 0, "copy {k} at place {place}");
    }
}

/// The radial phase is seeded by the array place: each array copy its own phase (ramp off), and a
/// mirror then an array keeps each mirrored pair together.
#[test]
fn the_radial_phase_follows_the_array_place() {
    let (_, pts) = copies_of(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
        {"id": "d", "kind": "displace", "mode": "radial", "amplitude": 60, "progressive": false},
    ]));
    let at_home: Vec<Vec<Point>> = pts.iter().enumerate().map(|(k, c)| c.iter().map(|p| *p - Vec2::new(150.0 * k as f64, 0.0)).collect()).collect();
    assert!(apart(&at_home[0], &at_home[1]) > 0.5 && apart(&at_home[1], &at_home[2]) > 0.5);
    let (out, pts) = copies_of(json!([
        {"id": "m", "kind": "mirror", "offset": 60},
        {"id": "a", "kind": "linearArray", "count": 3, "offsetY": 200},
        {"id": "d", "kind": "displace", "mode": "radial", "amplitude": 60, "progressive": false},
    ]));
    let pairs = mirror_pairs(&out, &pts);
    assert_eq!(pairs.len(), 3, "{pairs:?}");
    assert!(pairs.iter().all(|p| p.2 < 1e-9), "{pairs:?}");
    // Without a copy (on the object, or a public `shape_path`), the phase of place 0.
    let alone = Field::new(&radial(), Point::new(50.0, 50.0), None, Frame::IDENTITY).displace(&lopsided()).unwrap();
    let first = Field::new(&radial(), Point::new(50.0, 50.0), Some((0, 3)), Frame::IDENTITY).displace(&lopsided()).unwrap();
    assert_eq!(alone, first);
}

/// A copy's frame: a turned copy reads its ripple turned with it, a reflected one mirrored.
#[test]
fn the_frame_turns_and_reflects_the_ripple() {
    use vectorcraft_geom::Affine;
    let p = Displace { amplitude: 60.0, ripple_count: 3, ..radial() };
    let sp = lopsided();
    let c = Point::new(50.0, 50.0);
    for t in [Affine::rotate_about(0.7, c), Affine::rotate_about(-2.0, c) * Affine::FLIP_X, Affine::scale_non_uniform(1.0, -1.0)] {
        let mean = (sp.anchors.iter().fold(Vec2::ZERO, |s, a| s + a.p.to_vec2()) / sp.anchors.len() as f64).to_point();
        let here = Field::new(&p, mean, None, Frame::IDENTITY).displace(&sp).unwrap();
        let moved: Vec<Point> = sp.anchors.iter().map(|a| t * a.p).collect();
        let centre = moved.iter().fold(Vec2::ZERO, |s, q| s + q.to_vec2()) / moved.len() as f64;
        let there = Field::new(&p, centre.to_point(), None, Frame::of(t)).displace(&SubPath::polyline(&moved, true)).unwrap();
        let mapped: Vec<Point> = here.anchors.iter().map(|a| t * a.p).collect();
        let got: Vec<Point> = there.anchors.iter().map(|a| a.p).collect();
        assert!(apart(&mapped, &got) < 1e-9, "{t:?}: {}", apart(&mapped, &got));
    }
    assert_eq!(Frame::of(Affine::scale(0.0)), Frame::IDENTITY);
}
