//! Tests for the circular, grid, mirror, transformEach and randomize kinds and for stacking.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect};

use crate::*;

const GOLDEN: [[f64; 6]; 3] = [
    [1.1790970405697099, 0.21882812019188028, -0.21882812019188028, 1.1790970405697099, 2.215132279749602, -0.7290862720023759],
    [1.0306741826557329, -0.15555993206462243, 0.15555993206462243, 1.0306741826557329, 4.493668666001769, -0.460021802814075],
    [0.9987873579110254, -0.4587042465810462, 0.4587042465810462, 0.9987873579110254, -8.302763927659718, 1.733278058017457],
];
const RNG_GOLDEN: [f64; 3] = [0.29119620844873007, 0.2354680132764977, 0.35046256871122683];
const B: Rect = Rect::new(0.0, 0.0, 20.0, 10.0);

fn stack(mods: Value) -> Stack {
    parse(&json!({"v": 1, "stack": mods}).to_string()).unwrap()
}

fn eval(mods: Value) -> Output {
    evaluate(&stack(mods), B)
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-9
}

fn close(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn circular_full_turn_has_no_duplicate_and_original_is_copy_zero() {
    let out = eval(json!([{"id": "c", "kind": "circularArray", "count": 4, "radius": 50, "align": false}]));
    assert_eq!(out.instances.len(), 4);
    assert!(close(out.instances[0].transform, Affine::IDENTITY));
    let pivot = B.center();
    // Ring centre is 50 left of the original; copies sit a quarter turn apart (clockwise on screen).
    let c = Point::new(pivot.x - 50.0, pivot.y);
    let want = [(50.0, 0.0), (0.0, 50.0), (-50.0, 0.0), (0.0, -50.0)];
    for (k, (dx, dy)) in want.iter().enumerate() {
        assert!(near(out.instances[k].transform * pivot, Point::new(c.x + dx, c.y + dy)), "{k}");
    }
    assert!(out.guides.contains(&Guide::Circle { center: c, radius: 50.0 }));
}

#[test]
fn circular_partial_sweep_reaches_the_end_and_align_turns_copies() {
    let out = eval(json!([{"kind": "circularArray", "count": 3, "radius": 10, "sweep": 180, "align": true}]));
    let pivot = B.center();
    let last = out.instances[2].transform;
    // Last copy is half a turn on: pivot lands 20 to the left, the copy is turned 180 degrees.
    assert!(near(last * pivot, Point::new(pivot.x - 20.0, pivot.y)));
    let p = last * Point::new(pivot.x + 1.0, pivot.y);
    assert!(near(p, Point::new(pivot.x - 21.0, pivot.y)), "{p:?}");
    let out = eval(json!([{"kind": "circularArray", "count": 1, "sweep": 90}]));
    assert!(close(out.instances[0].transform, Affine::IDENTITY));
    // Centre offset moves the ring.
    let out = eval(json!([{"kind": "circularArray", "count": 2, "centerX": 7, "centerY": 3, "align": false}]));
    assert!(near(out.instances[0].transform * pivot, Point::new(pivot.x + 7.0, pivot.y + 3.0)));
}

#[test]
fn grid_gap_is_between_bounds() {
    let out = eval(json!([{"kind": "gridArray", "rows": 2, "cols": 3, "gapX": 5, "gapY": 2}]));
    assert_eq!(out.instances.len(), 6);
    for (k, i) in out.instances.iter().enumerate() {
        let (r, c) = (k / 3, k % 3);
        assert!(close(i.transform, Affine::translate((25.0 * c as f64, 12.0 * r as f64))), "{k}");
        assert_eq!(i.stage_indices, vec![k as u32]);
    }
    assert_eq!(out.guides.len(), 4);
    assert!(out.guides.contains(&Guide::Line { from: Point::new(0.0, 0.0), to: Point::new(70.0, 0.0) }));
}

#[test]
fn grid_over_the_cap_is_not_allocated() {
    let t = Instant::now();
    let out = eval(json!([
        {"id": "a", "kind": "linearArray", "count": 10},
        {"id": "g", "kind": "gridArray", "rows": 1000, "cols": 1000},
    ]));
    assert_eq!(out.instances.len(), 10);
    assert!(out.warnings.iter().any(|w| w.modifier.as_deref() == Some("g")));
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn mirror_variants() {
    let pivot = B.center();
    let p = Point::new(pivot.x + 3.0, pivot.y + 4.0);
    let v = eval(json!([{"kind": "mirror", "axis": "vertical"}]));
    assert_eq!(v.instances.len(), 2);
    assert!(near(v.instances[1].transform * p, Point::new(pivot.x - 3.0, pivot.y + 4.0)));
    let h = eval(json!([{"kind": "mirror", "axis": "horizontal", "offset": 10}]));
    assert!(near(h.instances[1].transform * p, Point::new(pivot.x + 3.0, pivot.y + 16.0)));
    let off = eval(json!([{"kind": "mirror", "keepOriginal": false}]));
    assert_eq!(off.instances.len(), 1);
    assert!(near(off.instances[0].transform * p, Point::new(pivot.x - 3.0, pivot.y + 4.0)));
    let both = eval(json!([{"kind": "mirror", "axis": "both"}]));
    assert_eq!(both.instances.len(), 4);
    assert_eq!(both.guides.len(), 2);
    let pts: Vec<Point> = both.instances.iter().map(|i| i.transform * p).collect();
    for want in [(3.0, 4.0), (-3.0, 4.0), (3.0, -4.0), (-3.0, -4.0)] {
        assert!(pts.iter().any(|q| near(*q, Point::new(pivot.x + want.0, pivot.y + want.1))), "{want:?}");
    }
    let three = eval(json!([{"kind": "mirror", "axis": "both", "keepOriginal": false}]));
    assert_eq!(three.instances.len(), 3);
}

#[test]
fn transform_each_progressive_ramps_with_t() {
    let out = eval(json!([
        {"kind": "linearArray", "count": 5, "offsetX": 100},
        {"kind": "transformEach", "offsetY": 40, "rotate": 90, "scaleX": 3, "progressive": true},
    ]));
    let pivot = B.center();
    assert_eq!(out.instances.len(), 5);
    // t = 0: untouched.
    assert!(close(out.instances[0].transform, Affine::IDENTITY));
    // t = 1: moved 40 down in its frame, turned 90 degrees and 3x wide about its own pivot.
    let last = out.instances[4].transform;
    assert!(near(last * pivot, Point::new(pivot.x + 400.0, pivot.y + 40.0)));
    let p = last * Point::new(pivot.x + 1.0, pivot.y);
    assert!(near(p, Point::new(pivot.x + 400.0, pivot.y + 40.0 + 3.0)), "{p:?}");
    // t = 0.5: half the offset.
    assert!(near(out.instances[2].transform * pivot, Point::new(pivot.x + 200.0, pivot.y + 20.0)));
    // Not progressive: everyone gets the full amount.
    let out = eval(json!([{"kind": "linearArray", "count": 2}, {"kind": "transformEach", "offsetY": 5}]));
    assert!(near(out.instances[0].transform * pivot, Point::new(pivot.x, pivot.y + 5.0)));
    // A single instance has t = 0.
    let out = eval(json!([{"kind": "transformEach", "offsetY": 5, "progressive": true}]));
    assert!(close(out.instances[0].transform, Affine::IDENTITY));
}

#[test]
fn transform_each_jitter_is_seeded() {
    let m = |seed: u32| json!([{"kind": "linearArray", "count": 4}, {"kind": "transformEach", "offsetX": 10, "jitter": 0.5, "jitterSeed": seed}]);
    let a = eval(m(1));
    assert_eq!(a, eval(m(1)));
    assert_ne!(a.instances[1].transform, eval(m(2)).instances[1].transform);
    let pivot = B.center();
    for i in &a.instances {
        let dx = (i.transform * pivot).x - pivot.x;
        let k = i.index as f64 * 100.0;
        assert!((dx - k).abs() >= 5.0 - 1e-9 && (dx - k).abs() <= 15.0 + 1e-9, "{dx}");
    }
}

#[test]
fn randomize_is_deterministic_with_golden_values() {
    let out = eval(json!([
        {"kind": "linearArray", "count": 3, "offsetX": 0},
        {"kind": "randomize", "position": 10, "rotation": 30, "scale": 0.2, "seed": 7},
    ]));
    let again = eval(json!([
        {"kind": "linearArray", "count": 3, "offsetX": 0},
        {"kind": "randomize", "position": 10, "rotation": 30, "scale": 0.2, "seed": 7},
    ]));
    assert_eq!(out, again);
    let got: Vec<[f64; 6]> = out.instances.iter().map(|i| i.transform.as_coeffs()).collect();
    let golden: [[f64; 6]; 3] = GOLDEN;
    for (g, w) in got.iter().zip(golden) {
        for (a, b) in g.iter().zip(w) {
            assert!((a - b).abs() < 1e-12, "{got:?}");
        }
    }
    // Other seed, other numbers; amplitudes bound the result.
    let other = eval(json!([{"kind": "linearArray", "count": 3, "offsetX": 0}, {"kind": "randomize", "position": 10, "seed": 8}]));
    assert_ne!(other.instances[1].transform, out.instances[1].transform);
    // A copy keeps its numbers when copies are added after it.
    let more = eval(json!([
        {"kind": "linearArray", "count": 6, "offsetX": 0},
        {"kind": "randomize", "position": 10, "rotation": 30, "scale": 0.2, "seed": 7},
    ]));
    assert_eq!(more.instances[1].transform, out.instances[1].transform);
    let pivot = B.center();
    for i in &more.instances {
        let d = i.transform * pivot - pivot;
        assert!(d.x.abs() <= 10.0 + 1e-9 && d.y.abs() <= 10.0 + 1e-9);
    }
}

#[test]
fn per_copy_stages_vary_every_copy_wherever_they_sit() {
    let rand = json!({"kind": "randomize", "position": 10, "rotation": 30, "scale": 0.2, "seed": 3});
    let each = json!({"kind": "transformEach", "rotate": 40, "progressive": true});
    let lin = json!({"kind": "linearArray", "count": 4, "offsetX": 100});
    let mir = json!({"kind": "mirror", "axis": "vertical", "offset": 60});
    for mods in [
        json!([rand.clone(), lin.clone()]),
        json!([lin.clone(), rand.clone()]),
        json!([rand.clone(), lin.clone(), mir.clone()]),
        json!([each.clone(), lin.clone()]),
        json!([lin.clone(), each.clone(), mir.clone()]),
    ] {
        let out = eval(mods.clone());
        let n = out.instances.len();
        assert!(n >= 4, "{mods}");
        // Strip each copy's array placement: what's left is its own per-copy transform.
        let own: Vec<[f64; 4]> = out
            .instances
            .iter()
            .map(|i| {
                let c = i.transform.as_coeffs();
                [c[0], c[1], c[2], c[3]]
            })
            .collect();
        for a in 0..n {
            for b in (a + 1)..n {
                let same = own[a].iter().zip(&own[b]).all(|(x, y)| (x.abs() - y.abs()).abs() < 1e-9);
                assert!(!same, "copies {a} and {b} got the same values in {mods}");
            }
        }
    }
    // The order of a per-copy row among the arrays doesn't change the result.
    let a = eval(json!([rand.clone(), lin.clone()]));
    let b = eval(json!([lin, rand]));
    assert!(a.instances.iter().zip(&b.instances).all(|(x, y)| close(x.transform, y.transform)));
}

#[test]
fn rng_golden_stream() {
    let mut s = kinds::rng_for_tests(7, 0);
    let v: Vec<f64> = (0..3).map(|_| s.signed()).collect();
    assert!(v.iter().all(|x| (-1.0..1.0).contains(x)));
    assert_eq!(v, RNG_GOLDEN);
}

#[test]
fn grid_x_circular_x_transform_each_composition() {
    let out = eval(json!([
        {"id": "g", "kind": "gridArray", "rows": 2, "cols": 3, "gapX": 0, "gapY": 0},
        {"id": "c", "kind": "circularArray", "count": 4, "radius": 100, "align": false},
        {"id": "t", "kind": "transformEach", "rotate": 10, "progressive": true},
    ]));
    assert_eq!(out.instances.len(), 24);
    let pivot = B.center();
    for (i, inst) in out.instances.iter().enumerate() {
        assert_eq!(inst.index as usize, i);
        assert_eq!(inst.stage_indices.len(), 2);
        // index = k_circular * n_grid + k_grid.
        assert_eq!(i, inst.stage_indices[1] as usize * 6 + inst.stage_indices[0] as usize);
    }
    // Instance 8: grid copy 2 (row 0, col 2), circular copy 1 (quarter turn), t = 8/23.
    let i8 = &out.instances[8];
    assert_eq!(i8.stage_indices, vec![2, 1]);
    let ring = Affine::translate((-100.0, 100.0)); // copy 1 on a 100 ring: a quarter turn clockwise about the centre 100 to the left
    let t: f64 = 8.0 / 23.0;
    let want = ring * Affine::translate((40.0, 0.0)) * Affine::rotate_about((10.0 * t).to_radians(), pivot);
    assert!(close(i8.transform, want), "{:?} vs {:?}", i8.transform, want);
    assert!(close(out.instances[0].transform, Affine::IDENTITY));
    assert_eq!(out.guides.iter().filter(|g| matches!(g, Guide::Circle { .. })).count(), 1);
}

#[test]
fn the_cap_keeps_the_stages_that_fit_and_never_hangs() {
    let t = Instant::now();
    let out = eval(json!([
        {"id": "a", "kind": "gridArray", "rows": 100, "cols": 10},
        {"id": "b", "kind": "circularArray", "count": 10},
        {"id": "c", "kind": "circularArray", "count": 2},
        {"id": "d", "kind": "randomize", "position": 5},
    ]));
    assert_eq!(out.instances.len(), 10_000);
    assert!(out.warnings.iter().any(|w| w.modifier.as_deref() == Some("c")), "{:?}", out.warnings);
    assert!(out.instances.iter().all(|i| i.transform.is_finite()));
    let many: Vec<Value> = (0..64).map(|i| json!({"id": format!("m{i}"), "kind": "gridArray", "rows": 1000, "cols": 1000})).collect();
    let out = eval(Value::Array(many));
    assert_eq!(out.instances.len(), 1);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
}

#[test]
fn junk_params_are_clamped_in_every_kind() {
    let mut s = Stack::default();
    let junk = f64::NAN;
    let inf = f64::INFINITY;
    for kind in [
        ModifierKind::CircularArray(CircularArray {
            count: 5,
            radius: inf,
            start_angle: junk,
            sweep: 1e300,
            center_x: junk,
            center_y: inf,
            align: true,
        }),
        ModifierKind::GridArray(GridArray { rows: 3, cols: 3, gap_x: inf, gap_y: junk }),
        ModifierKind::Mirror(Mirror { axis: MirrorAxis::Both, offset: inf, keep_original: true }),
        ModifierKind::TransformEach(TransformEach {
            offset_x: inf,
            offset_y: junk,
            rotate: 1e308,
            scale_x: inf,
            scale_y: junk,
            progressive: true,
            jitter: junk,
            jitter_seed: 3,
            rotate_expr: Some("1/0 + i".into()),
            scale_x_expr: Some("value ^ 1e9".into()),
            offset_y_expr: Some("((".into()),
            ..Default::default()
        }),
        ModifierKind::Randomize(Randomize {
            position: inf,
            rotation: junk,
            scale: 1e308,
            seed: 1,
            position_expr: Some("sqrt(-1)".into()),
            rotation_expr: Some("-1e308 * i".into()),
            ..Default::default()
        }),
    ] {
        s.modifiers.push(Modifier {
            id: format!("m{}", s.modifiers.len()),
            enabled: true,
            kind,
            extra: Default::default(),
            bind: Default::default(),
        });
    }
    for bounds in [B, Rect::new(f64::NAN, 0.0, 1.0, f64::INFINITY)] {
        let out = evaluate(&s, bounds);
        assert!(!out.instances.is_empty());
        assert!(out.instances.iter().all(|i| i.transform.is_finite()));
        assert!(!out.warnings.is_empty());
    }
}

#[test]
fn new_params_round_trip() {
    let m = Modifier::new("m1", "transformEach").unwrap();
    let v = m.to_value();
    assert_eq!(v["jitter"], json!(0.0));
    assert_eq!(Modifier::from_value(&v).unwrap(), m);
    let mut mirror = Modifier::new("m2", "mirror").unwrap();
    mirror.set_props(json!({"axis": "both"}).as_object().unwrap()).unwrap();
    assert_eq!(mirror.to_value()["axis"], json!("both"));
}
