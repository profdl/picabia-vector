//! PIC20.4c: displace's wave, jitter and fractal patterns and the shared direction.

// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, Vec2};

use super::displace::{Field, Frame, jitter_start, normals, waveform};
use super::displace_tests::{apart, bits, circle, copies_of, lopsided, mirror_pairs, path_node, square, stack};
use super::fractal::{count, seeded};
use super::{GeometryOp, GeometryRow, MAX_GEOMETRY_ANCHORS, RowAt, Shaper, shape_path};
use crate::model::{Displace, DisplaceDirection, DisplaceMode, ModifierKind, Waveform};
use crate::{Output, evaluate};

fn wave() -> Displace {
    Displace { mode: DisplaceMode::Wave, progressive: false, ..Displace::default() }
}

fn jitter() -> Displace {
    Displace { mode: DisplaceMode::Jitter, progressive: false, ..Displace::default() }
}

fn fractal() -> Displace {
    Displace { mode: DisplaceMode::Fractal, progressive: false, ..Displace::default() }
}

/// A horizontal row of `n` points from x 0 to 100 at y 0, open.
fn row(n: usize) -> SubPath {
    let pts: Vec<Point> = (0..n).map(|i| Point::new(100.0 * i as f64 / (n - 1) as f64, 0.0)).collect();
    SubPath::polyline(&pts, false)
}

fn moved(p: &Displace, sp: &SubPath) -> PathData {
    shape_path(&PathData::new(vec![sp.clone()]), &GeometryOp::Displace(p.clone()))
}

#[test]
fn the_new_fields_default_and_clamp_to_tlproceduruals_ranges() {
    let ModifierKind::Displace(d) = crate::Modifier::new("m", "displace").unwrap().kind else { panic!() };
    // WAVE_DEFAULTS, RANDOMIZE_DEFAULTS' minDistance, SUBDIVISION_GROWTH_DEFAULTS.
    assert_eq!((d.direction, d.angle), (DisplaceDirection::Own, 0.0));
    assert_eq!((d.waveform, d.frequency, d.phase), (Waveform::Sine, 2.0, 0.0));
    assert_eq!(d.jitter_min, 0.0);
    assert_eq!((d.steps, d.decay, d.wander, d.edge_variance, d.normal_bias, d.inward), (4, 0.45, 0.15, 0.0, 0.5, false));
    let params = ModifierKind::Displace(d).params();
    for key in
        ["direction", "angle", "waveform", "frequency", "phase", "jitterMin", "steps", "decay", "wander", "edgeVariance", "normalBias", "inward"]
    {
        assert!(params.contains_key(key), "{key} in {params:?}");
    }
    let out = evaluate(
        &stack(json!([{"id": "d", "kind": "displace", "mode": "fractal", "amplitude": 1e9, "angle": -1e9, "frequency": 0, "phase": 999,
            "jitterMin": -5, "steps": 99, "decay": 2, "wander": -1, "edgeVariance": 7, "normalBias": f64::MAX}])),
        Rect::new(0.0, 0.0, 100.0, 100.0),
    );
    let GeometryOp::Displace(p) = &out.geometry[0].op else { panic!() };
    // Subdivision Growth's displacement reaches 200.
    assert_eq!((p.amplitude, p.angle, p.frequency, p.phase, p.jitter_min), (200.0, -360.0, 0.1, 360.0, -1.0));
    assert_eq!((p.steps, p.decay, p.wander, p.edge_variance, p.normal_bias), (8, 0.5, 0.0, 1.0, 1.0));
    assert!(out.warnings.len() >= 10, "{:?}", out.warnings);
    // The other patterns' amplitude stops at 100.
    let out = evaluate(&stack(json!([{"id": "d", "kind": "displace", "mode": "wave", "amplitude": 150}])), Rect::new(0.0, 0.0, 1.0, 1.0));
    let GeometryOp::Displace(p) = &out.geometry[0].op else { panic!() };
    assert_eq!(p.amplitude, 100.0);
    // A direction or waveform that isn't one doesn't read: the row is kept but ignored.
    for bad in [json!({"direction": "up"}), json!({"waveform": "zigzag"}), json!({"steps": "many"})] {
        let mut row = json!({"id": "d", "kind": "displace"});
        row.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        assert!(!evaluate(&stack(json!([row])), Rect::new(0.0, 0.0, 1.0, 1.0)).has_geometry(), "{bad}");
    }
}

/// A stack saved before the new patterns (only the old fields) reads the same params and draws
/// the same bits: the new direction defaults to each pattern's own.
#[test]
fn old_saved_spatial_and_radial_stacks_draw_unchanged() {
    let old = json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
        {"id": "d", "kind": "displace", "mode": "radial", "amplitude": 60, "scale": 0.05, "octaves": 3, "rippleCount": 3, "seed": 42,
         "progressive": true, "progressiveAmount": 1.0, "progressiveReverse": false},
    ]);
    let (_, pts) = copies_of(old);
    let p = pts[2][3];
    assert_eq!((p.x.to_bits(), p.y.to_bits()), OLD_RADIAL_BITS, "radial {p:?}");
    let (_, pts) = copies_of(json!([{"id": "d", "kind": "displace", "mode": "spatial", "amplitude": 30, "scale": 0.05, "octaves": 3, "seed": 7}]));
    let p = pts[0][5];
    assert_eq!((p.x.to_bits(), p.y.to_bits()), OLD_SPATIAL_BITS, "spatial {p:?}");
    // And the old fields alone read as the defaults of the new ones.
    let s = stack(json!([{"id": "d", "kind": "displace", "mode": "radial"}]));
    let ModifierKind::Displace(d) = &s.modifiers[0].kind else { panic!() };
    assert_eq!(d.direction, DisplaceDirection::Own);
}

/// Pinned before PIC20.4c (worked out on the build before the new patterns landed).
const OLD_RADIAL_BITS: (u64, u64) = (4644971660134963131, 4635140693955401868);
const OLD_SPATIAL_BITS: (u64, u64) = (4617371838361822821, 4637049005661068134);

#[test]
fn waveforms_are_tlproceduruals() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    for i in 0..=40 {
        let x = f64::from(i) / 8.0 - 2.0;
        let m = ((x % 1.0) + 1.0) % 1.0;
        assert!(close(waveform(Waveform::Sine, x), (x * std::f64::consts::TAU).sin()), "{x}");
        assert_eq!(waveform(Waveform::Square, x), if m < 0.5 { 1.0 } else { -1.0 }, "{x}");
        assert!(close(waveform(Waveform::Sawtooth, x), m * 2.0 - 1.0), "{x}");
        let tri = waveform(Waveform::Triangle, x);
        assert!((-1.0..=1.0).contains(&tri));
    }
    assert_eq!(waveform(Waveform::Triangle, 0.25), 1.0);
    assert_eq!(waveform(Waveform::Triangle, 0.75), -1.0);
    assert_eq!(waveform(Waveform::Triangle, 0.5), 0.0);
    assert_eq!(waveform(Waveform::Sine, f64::NAN), 0.0);
    assert_eq!(waveform(Waveform::Square, f64::INFINITY), 0.0);
}

/// tlprocedural's `wavePolygon`: across the width (its horizontal axis), each point moves in y by
/// amplitude · wave(t · frequency + phase/360), t its place across the width.
#[test]
fn a_wave_moves_across_the_objects_width() {
    let sp = row(9);
    let p = Displace { amplitude: 20.0, frequency: 2.0, phase: 90.0, ..wave() };
    let d = moved(&p, &sp);
    for (a, b) in sp.anchors.iter().zip(&d.subpaths[0].anchors) {
        let t = a.p.x / 100.0;
        let want = 20.0 * ((t * 2.0 + 0.25) * std::f64::consts::TAU).sin();
        assert_eq!(b.p.x, a.p.x, "moves in y only");
        assert!((b.p.y - want).abs() < 1e-9, "{} vs {want}", b.p.y);
    }
    // Pinned off the quarter cycles (phase 30°), so the bits are the sine's own.
    let a = moved(&Displace { phase: 30.0, ..p.clone() }, &sp).subpaths[0].anchors[3].p;
    assert_eq!((a.x.to_bits(), a.y.to_bits()), WAVE_BITS, "wave {a:?}");
    // Direction x: along the height, moving in x (tlprocedural's vertical axis, run up the height so
    // that angle 0° is x).
    let column = SubPath::polyline(&sp.anchors.iter().map(|a| Point::new(0.0, a.p.x)).collect::<Vec<_>>(), false);
    let dx = moved(&Displace { direction: DisplaceDirection::X, ..p.clone() }, &column);
    for (a, b) in column.anchors.iter().zip(&dx.subpaths[0].anchors) {
        let want = 20.0 * (((1.0 - a.p.y / 100.0) * 2.0 + 0.25) * std::f64::consts::TAU).sin();
        assert!(b.p.y == a.p.y && (b.p.x - want).abs() < 1e-9);
    }
    // Angle 90° is y.
    let ninety = moved(&Displace { direction: DisplaceDirection::Angle, angle: 90.0, ..p.clone() }, &sp);
    assert!(apart_paths(&ninety, &d) < 1e-9);
    // A wave across a column has nothing to run across: left alone (tlprocedural's 0.001).
    assert_eq!(moved(&p, &column), PathData::new(vec![column.clone()]));
    // The square wave and the ramp's share: every anchor at ±amplitude.
    let sq = moved(&Displace { waveform: Waveform::Square, ..p.clone() }, &sp);
    assert!(sq.subpaths[0].anchors.iter().all(|a| a.p.y.abs() == 20.0));
}

const WAVE_BITS: (u64, u64) = (4630474473122824192, 13848940544144125926);

fn apart_paths(a: &PathData, b: &PathData) -> f64 {
    let pa: Vec<Point> = a.subpaths.iter().flat_map(|s| &s.anchors).map(|x| x.p).collect();
    let pb: Vec<Point> = b.subpaths.iter().flat_map(|s| &s.anchors).map(|x| x.p).collect();
    apart(&pa, &pb)
}

/// The normal points out of the shape whichever way it is wound; a wave along it runs round the
/// outline.
#[test]
fn normals_point_out_and_a_wave_runs_along_the_outline() {
    let sq = square();
    let centre = Point::new(50.0, 50.0);
    let reversed = SubPath::new(sq.anchors.iter().rev().copied().collect(), true);
    for sp in [&sq, &reversed, &circle()] {
        let c = if sp.anchors.len() == 4 { centre } else { Point::new(100.0, 100.0) };
        for (a, n) in sp.anchors.iter().zip(normals(sp)) {
            assert!((n.x * n.x + n.y * n.y - 1.0).abs() < 1e-9);
            assert!(n.dot(a.p - c) > 0.0, "{n:?} at {:?}", a.p);
        }
    }
    // Jitter along the normal, least = all of it: every anchor of the square moves out by the
    // amplitude along its corner's bisector.
    let p = Displace { amplitude: 10.0, jitter_min: 1.0, direction: DisplaceDirection::Normal, ..jitter() };
    let d = moved(&p, &sq);
    for (a, b) in sq.anchors.iter().zip(&d.subpaths[0].anchors) {
        let off = b.p - a.p;
        assert!(((off.x * off.x + off.y * off.y).sqrt() - 10.0).abs() < 1e-9);
        assert!(off.dot(a.p - centre) > 0.0);
    }
    // A wave along the normal: the circle's four anchors at quarter places, phase 90°, one cycle:
    // cos(2π·k/4) of the amplitude out.
    let w = Displace { amplitude: 10.0, frequency: 1.0, phase: 90.0, direction: DisplaceDirection::Normal, ..wave() };
    let c = circle();
    let d = moved(&w, &c);
    for (k, (a, b)) in c.anchors.iter().zip(&d.subpaths[0].anchors).enumerate() {
        let out = (b.p - a.p).dot((a.p - Point::new(100.0, 100.0)) / 100.0);
        let want = 10.0 * (std::f64::consts::TAU * k as f64 / 4.0).cos();
        assert!((out - want).abs() < 1e-6, "anchor {k}: {out} vs {want}");
    }
}

/// tlprocedural's Randomize Points: each anchor along its direction from the centre by
/// min + rng · (max - min), its LCG stepped once per anchor.
#[test]
fn jitter_moves_each_anchor_out_by_a_seeded_amount() {
    let c = circle();
    let p = Displace { amplitude: 24.0, ..jitter() };
    let d = Field::new(&p, Point::new(100.0, 100.0), None, Frame::IDENTITY).displace(&c).unwrap();
    let mut state: u64 = 42;
    for (a, b) in c.anchors.iter().zip(&d.anchors) {
        state = (state * 1_103_515_245 + 12_345) & 0x7fff_ffff;
        let want = 24.0 * state as f64 / f64::from(0x7fff_ffff);
        let (r0, r1) = (a.p - Point::new(100.0, 100.0), b.p - Point::new(100.0, 100.0));
        let (l0, l1) = ((r0.x * r0.x + r0.y * r0.y).sqrt(), (r1.x * r1.x + r1.y * r1.y).sqrt());
        assert!((l1 - l0 - want).abs() < 1e-9, "{} vs {want}", l1 - l0);
        assert!(r0.cross(r1).abs() < 1e-6, "along the radius");
        assert!(((b.h_in - b.p) - (a.h_in - a.p)).hypot() < 1e-9, "handles with it");
    }
    let a = d.anchors[1].p;
    assert_eq!((a.x.to_bits(), a.y.to_bits()), JITTER_BITS, "jitter {a:?}");
    // Least -1: either way, within the amplitude.
    let both = Displace { amplitude: 24.0, jitter_min: -1.0, ..jitter() };
    let sp = lopsided();
    let d = Field::new(&both, Point::new(50.0, 50.0), None, Frame::IDENTITY).displace(&sp).unwrap();
    let (mut ins, mut outs) = (0, 0);
    for (a, b) in sp.anchors.iter().zip(&d.anchors) {
        let (l0, l1) = ((a.p - Point::new(50.0, 50.0)).hypot(), (b.p - Point::new(50.0, 50.0)).hypot());
        assert!((l1 - l0).abs() <= 24.0 + 1e-9);
        ins += usize::from(l1 < l0);
        outs += usize::from(l1 > l0);
    }
    assert!(ins > 0 && outs > 0, "{ins} in, {outs} out");
    // Never through the centre: a big pull in stops just short of it.
    let near = SubPath::polyline(&[Point::new(51.0, 50.0), Point::new(40.0, 50.0)], false);
    let pull = Displace { amplitude: 100.0, jitter_min: -1.0, seed: 3, ..jitter() };
    let d = Field::new(&pull, Point::new(50.0, 50.0), None, Frame::IDENTITY).displace(&near).unwrap();
    for (a, b) in near.anchors.iter().zip(&d.anchors) {
        assert!((b.p - Point::new(50.0, 50.0)).dot(a.p - Point::new(50.0, 50.0)) > 0.0, "same side");
    }
    // Each copy its own stream: copy 0 is tlprocedural's seed, the others the crate's stream.
    assert_eq!(jitter_start(42, 0), 42);
    assert_eq!(jitter_start(0, 0), 1, "seed || 1");
    assert_ne!(jitter_start(42, 1), jitter_start(42, 2));
    // Along x: only x moves.
    let x = moved(&Displace { direction: DisplaceDirection::X, ..p }, &c);
    assert!(c.anchors.iter().zip(&x.subpaths[0].anchors).all(|(a, b)| a.p.y == b.p.y && b.p.x >= a.p.x));
}

const JITTER_BITS: (u64, u64) = (4636737291354636288, 13846584821489461008);

/// tlprocedural's `seededRandom` (mulberry32), worked by hand for two inputs.
#[test]
fn seeded_is_mulberry32() {
    // t = 42 ^ 0; t = imul(t ^ t>>>15, t | 1) = 42·43 = 1806; t ^= t + imul(t ^ t>>>7, t | 61)…
    let mut t: u32 = 42;
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    assert_eq!(seeded(42, 0), f64::from(t ^ (t >> 14)) / 4_294_967_296.0);
    assert_eq!(seeded(42, 0), SEEDED_42_0);
    for c in 0..1000 {
        assert!((0.0..1.0).contains(&seeded(7, c)));
    }
    assert_ne!(seeded(1, 5), seeded(2, 5));
}

const SEEDED_42_0: f64 = 0.000_774_848_042_055_964_5;

/// Subdivision growth: every segment split, the new anchor pushed out from the centre by the
/// amplitude, `decay` times less each step; the old anchors stay where they were.
#[test]
fn fractal_splits_every_segment_and_pushes_the_new_anchor_out() {
    let sq = square();
    let p = Displace { amplitude: 30.0, steps: 1, wander: 0.0, normal_bias: 0.0, ..fractal() };
    let d = moved(&p, &sq);
    let a = &d.subpaths[0].anchors;
    assert_eq!(a.len(), 8);
    assert_eq!(count(&sq, 1), 8);
    for k in 0..4 {
        assert_eq!(a[2 * k].p, sq.anchors[k].p, "the old anchors stay");
        let mid = sq.anchors[k].p.midpoint(sq.anchors[(k + 1) % 4].p);
        let out = (mid - Point::new(50.0, 50.0)) / 50.0;
        assert!((a[2 * k + 1].p - (mid + out * 30.0)).hypot() < 1e-9, "{:?}", a[2 * k + 1].p);
        // Straight segments stay straight: no handles.
        assert!(a[2 * k + 1].h_in == a[2 * k + 1].p && a[2 * k + 1].h_out == a[2 * k + 1].p);
    }
    // Two steps: 16 anchors, the second push 0.45 of the first.
    let two = moved(&Displace { steps: 2, ..p.clone() }, &sq);
    assert_eq!(two.subpaths[0].anchors.len(), 16);
    assert_eq!(count(&sq, 2), 16);
    // Inward: pushed in.
    let inward = moved(&Displace { inward: true, ..p.clone() }, &sq);
    assert!((inward.subpaths[0].anchors[1].p - Point::new(50.0, 50.0)).hypot() < 50.0);
    // An open path grows its segments only: (n - 1) · 2^steps + 1.
    let open = moved(&Displace { steps: 3, ..p.clone() }, &row(5));
    assert_eq!(open.subpaths[0].anchors.len(), 4 * 8 + 1);
    assert_eq!(count(&row(5), 3), 33);
    // Curves stay curves: the circle's new anchors sit on the old curve before their push (zero
    // amplitude does nothing at all; a tiny one moves them a hair).
    let c = circle();
    let tiny = moved(&Displace { amplitude: 1e-9, steps: 1, ..p.clone() }, &c);
    let bez = PathData::new(vec![c.clone()]).to_bezpath();
    let grown = tiny.to_bezpath();
    let area = |b: &vectorcraft_geom::BezPath| vectorcraft_geom::Shape::area(b);
    assert!((area(&bez) - area(&grown)).abs() < 1e-3 * area(&bez).abs(), "{} vs {}", area(&bez), area(&grown));
    // The default params, pinned (wander and the seeded split included).
    let d = moved(&Displace { edge_variance: 0.6, ..fractal() }, &lopsided());
    assert_eq!(d.subpaths[0].anchors.len(), 14 * 16);
    let a = d.subpaths[0].anchors[5].p;
    assert_eq!((a.x.to_bits(), a.y.to_bits()), FRACTAL_BITS, "fractal {a:?}");
}

const FRACTAL_BITS: (u64, u64) = (4636292046386374060, 4632247438135304470);

/// A fractal that would make more anchors than the cap is refused before it allocates: on its
/// own (no change), and through the shaper with a note.
#[test]
fn a_fractal_over_the_anchor_cap_is_refused_with_a_note() {
    let pts: Vec<Point> =
        (0..1000).map(|i| f64::from(i) / 1000.0 * std::f64::consts::TAU).map(|t| Point::new(100.0 * t.cos(), 100.0 * t.sin())).collect();
    let big = SubPath::polyline(&pts, true);
    let p = Displace { steps: 8, ..fractal() };
    assert!(count(&big, 8) > MAX_GEOMETRY_ANCHORS);
    assert_eq!(moved(&p, &big), PathData::new(vec![big.clone()]), "left alone");
    let rows = vec![GeometryRow { modifier: "m".into(), op: GeometryOp::Displace(p), at: RowAt::Source }];
    let out = Output { instances: vec![crate::Instance::identity()], geometry: rows, ..Output::default() };
    let mut sh = Shaper::new(&out);
    let mut n = path_node(big.clone());
    sh.source(&mut n);
    assert_eq!(n.path_data().unwrap().subpaths[0].anchors.len(), 1000);
    assert!(sh.notes.iter().any(|s| s.contains("Displace would make more than 200000 anchors")), "{:?}", sh.notes);
    // Fewer steps fit.
    let fits = vec![GeometryRow { modifier: "m".into(), op: GeometryOp::Displace(Displace { steps: 6, ..fractal() }), at: RowAt::Source }];
    let out = Output { instances: vec![crate::Instance::identity()], geometry: fits, ..Output::default() };
    let mut sh = Shaper::new(&out);
    let mut n = path_node(big);
    sh.source(&mut n);
    assert_eq!(n.path_data().unwrap().subpaths[0].anchors.len(), 64_000);
    assert!(sh.notes.is_empty(), "{:?}", sh.notes);
}

#[test]
fn every_pattern_is_deterministic_and_seeded() {
    let c = lopsided();
    for p in [wave(), jitter(), fractal(), Displace { direction: DisplaceDirection::Normal, ..fractal() }] {
        let one = moved(&p, &c);
        let two = moved(&p, &c);
        assert_eq!(bits(&one.subpaths[0]), bits(&two.subpaths[0]), "{:?}", p.mode);
        if p.mode != DisplaceMode::Wave {
            let other = moved(&Displace { seed: 7, ..p.clone() }, &c);
            assert_ne!(bits(&one.subpaths[0]), bits(&other.subpaths[0]), "{:?}", p.mode);
        } else {
            let other = moved(&Displace { phase: 45.0, ..p.clone() }, &c);
            assert_ne!(bits(&one.subpaths[0]), bits(&other.subpaths[0]));
        }
    }
}

#[test]
fn degenerate_geometry_and_junk_params_never_panic() {
    let weird = [
        SubPath::new(vec![], false),
        SubPath::polyline(&[Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::ZERO, Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(20.0, 0.0)], true),
        SubPath::polyline(&[Point::new(1e300, -1e300), Point::new(-1e300, 1e300), Point::new(1e300, 1e300)], true),
        SubPath::polyline(&[Point::new(f64::MAX, f64::MAX), Point::new(-f64::MAX, f64::MAX)], false),
        SubPath::new(vec![Anchor::with_handles(Point::ZERO, Point::new(5.0, 5.0), Point::new(-5.0, 5.0)), Anchor::corner(Point::ZERO)], true),
        SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(2.0, 0.0)], true),
        lopsided(),
    ];
    let junk = [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e308, -1e308, 0.0];
    let mut ops = vec![];
    for mode in [DisplaceMode::Wave, DisplaceMode::Jitter, DisplaceMode::Fractal, DisplaceMode::Spatial, DisplaceMode::Radial] {
        for direction in [DisplaceDirection::Own, DisplaceDirection::Normal, DisplaceDirection::X, DisplaceDirection::Y, DisplaceDirection::Angle] {
            for v in junk {
                ops.push(Displace {
                    mode,
                    direction,
                    progressive: false,
                    amplitude: if v == 0.0 { 50.0 } else { v },
                    angle: v,
                    frequency: v,
                    phase: v,
                    jitter_min: v,
                    decay: v,
                    wander: v,
                    edge_variance: v,
                    normal_bias: v,
                    steps: u32::MAX,
                    ..Displace::default()
                });
            }
        }
    }
    let mut moved_any = 0;
    for sp in &weird {
        for p in &ops {
            let rows = vec![GeometryRow { modifier: "m".into(), op: GeometryOp::Displace(p.clone()), at: RowAt::Copies }];
            let out = Output { instances: vec![crate::Instance::identity(); 2], geometry: rows, ..Output::default() };
            let mut sh = Shaper::new(&out);
            for k in 0..2 {
                let before = path_node(sp.clone());
                let mut n = before.clone();
                sh.copy_at(&mut n, k);
                let ok = n.path_data().unwrap().subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite());
                assert!(ok || !sp.anchors.iter().all(|a| a.p.is_finite()), "{p:?} on {sp:?}");
                moved_any += usize::from(n != before);
            }
            let _ = Field::new(p, Point::new(1.0, 1.0), Some((9, 3)), Frame::IDENTITY).spanning(f64::NAN, 5.0).displace(sp);
            let _ = shape_path(&PathData::new(vec![sp.clone()]), &GeometryOp::Displace(p.clone()));
        }
    }
    assert!(moved_any > 50, "only {moved_any} moved");
    // Degenerate cases left alone: a closed fractal with no area (tlprocedural's null), a wave
    // over a point.
    let flat = SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(20.0, 0.0)], true);
    assert_eq!(moved(&fractal(), &flat), PathData::new(vec![flat.clone()]));
    let dot = SubPath::polyline(&[Point::new(3.0, 3.0)], false);
    assert_eq!(moved(&wave(), &dot), PathData::new(vec![dot]));
}

/// Below an array: the ramp leaves the first copy alone; each copy its own jitter stream and
/// fractal seeds. A lone mirror: both halves the mirror image of each other (wave, jitter and
/// fractal read in each copy's frame, the reflected one's wander turning the other way).
#[test]
fn copies_ramp_and_mirrors_stay_symmetric() {
    let plain: Vec<Point> = lopsided().anchors.iter().map(|a| a.p).collect();
    for mode in ["wave", "jitter", "fractal"] {
        let (_, pts) = copies_of(json!([
            {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150},
            {"id": "d", "kind": "displace", "mode": mode, "amplitude": 20, "steps": 2},
        ]));
        assert_eq!(pts[0], plain, "{mode}: copy 0 unmoved");
        let home = |k: usize| pts[k].iter().map(|p| *p - Vec2::new(150.0 * k as f64, 0.0)).collect::<Vec<_>>();
        if mode == "fractal" {
            assert_eq!(pts[1].len(), 14 * 4);
        }
        assert_ne!(home(1), home(2), "{mode}");
        let (out, pts) =
            copies_of(json!([{"id": "m", "kind": "mirror"}, {"id": "d", "kind": "displace", "mode": mode, "amplitude": 20, "steps": 2}]));
        let pairs = mirror_pairs(&out, &pts);
        assert_eq!(pairs.len(), 1, "{mode}");
        assert!(pairs[0].2 < 1e-9, "{mode}: a mirror image, {pairs:?}");
        // Every direction, and the fractal's seeded split places, stay mirror images too.
        for direction in ["own", "normal", "x", "y", "angle"] {
            let (out, pts) = copies_of(json!([{"id": "m", "kind": "mirror"}, {"id": "d", "kind": "displace", "mode": mode, "amplitude": 20,
                "steps": 2, "direction": direction, "angle": 30, "edgeVariance": 0.7}]));
            let pairs = mirror_pairs(&out, &pts);
            assert_eq!(pairs.len(), 1, "{mode} {direction}");
            assert!(pairs[0].2 < 1e-9, "{mode} {direction}: a mirror image, {pairs:?}");
        }
    }
}

/// PIC20.4d: a wave round a closed outline takes a whole number of cycles (at least one), so it
/// meets itself where the outline starts; an open one keeps its cycles.
#[test]
fn a_wave_round_a_closed_outline_takes_whole_cycles() {
    use super::displace::whole_cycles;
    assert_eq!((whole_cycles(2.5), whole_cycles(2.4), whole_cycles(0.1), whole_cycles(10.0)), (3.0, 2.0, 1.0, 10.0));
    assert!(whole_cycles(f64::NAN).is_nan());
    let pts: Vec<Point> =
        (0..48).map(|i| f64::from(i) / 48.0 * std::f64::consts::TAU).map(|t| Point::new(100.0 * t.cos(), 100.0 * t.sin())).collect();
    let ring = SubPath::polyline(&pts, true);
    let at = |f: f64, sp: &SubPath| {
        let p = Displace { amplitude: 10.0, frequency: f, phase: 90.0, direction: DisplaceDirection::Normal, ..wave() };
        moved(&p, sp)
    };
    assert_eq!(bits(&at(2.5, &ring).subpaths[0]), bits(&at(3.0, &ring).subpaths[0]), "2.5 cycles round: 3");
    // No seam: the step from the last anchor back to the first is like any other step.
    let d = at(2.5, &ring);
    let off: Vec<f64> = ring.anchors.iter().zip(&d.subpaths[0].anchors).map(|(a, b)| (b.p - a.p).dot(a.p.to_vec2() / 100.0)).collect();
    let steps: Vec<f64> = (0..48).map(|i| (off[(i + 1) % 48] - off[i]).abs()).collect();
    let most = steps[..47].iter().copied().fold(0.0, f64::max);
    assert!(steps[47] <= most + 1e-9, "seam {} vs {most}", steps[47]);
    // Open: 2.5 cycles stay 2.5.
    let open = SubPath::polyline(&pts, false);
    assert_ne!(bits(&at(2.5, &open).subpaths[0]), bits(&at(3.0, &open).subpaths[0]));
}

/// PIC20.4d: x, y and angle are one turning family: angle 0° is x, 90° is y.
#[test]
fn angle_zero_is_x_and_ninety_is_y() {
    let sq = lopsided();
    for mode in [DisplaceMode::Wave, DisplaceMode::Jitter, DisplaceMode::Spatial, DisplaceMode::Fractal] {
        let base = Displace { mode, amplitude: 12.0, progressive: false, ..Displace::default() };
        let x = moved(&Displace { direction: DisplaceDirection::X, ..base.clone() }, &sq);
        let a0 = moved(&Displace { direction: DisplaceDirection::Angle, angle: 0.0, ..base.clone() }, &sq);
        assert_eq!(bits(&x.subpaths[0]), bits(&a0.subpaths[0]), "{mode:?}: 0° is x");
        let y = moved(&Displace { direction: DisplaceDirection::Y, ..base.clone() }, &sq);
        let a90 = moved(&Displace { direction: DisplaceDirection::Angle, angle: 90.0, ..base.clone() }, &sq);
        assert!(apart_paths(&y, &a90) < 1e-9, "{mode:?}: 90° is y");
        let a360 = moved(&Displace { direction: DisplaceDirection::Angle, angle: 360.0, ..base.clone() }, &sq);
        assert!(apart_paths(&x, &a360) < 1e-9, "{mode:?}: 360° is x");
    }
}

/// PIC20.4d: the normal's outward side comes from the curve's own winding, so a two-anchor lens
/// (its anchors enclose nothing) gets outward normals whichever way it runs, and a fractal along
/// the normal grows it out.
#[test]
fn a_two_anchor_lens_has_outward_normals_both_ways() {
    let lens = SubPath::new(
        vec![
            Anchor::with_handles(Point::new(0.0, 0.0), Point::new(30.0, 40.0), Point::new(30.0, -40.0)),
            Anchor::with_handles(Point::new(100.0, 0.0), Point::new(70.0, -40.0), Point::new(70.0, 40.0)),
        ],
        true,
    );
    let mut back = lens.clone();
    back.reverse();
    let centre = Point::new(50.0, 0.0);
    for sp in [&lens, &back] {
        for (a, n) in sp.anchors.iter().zip(normals(sp)) {
            assert!(n.dot(a.p - centre) > 0.99, "{n:?} at {:?}", a.p);
        }
        let p = Displace { amplitude: 10.0, jitter_min: 1.0, direction: DisplaceDirection::Normal, ..jitter() };
        let d = moved(&p, sp);
        for (a, b) in sp.anchors.iter().zip(&d.subpaths[0].anchors) {
            assert!((b.p - a.p).dot(a.p - centre) > 0.0, "out");
        }
    }
    // A three-anchor curved shape whose anchors lie on one line: the fractal still grows it (the
    // curve has area), out along the normal.
    let flat_curvy = SubPath::new(
        vec![
            Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 40.0), Point::new(0.0, -40.0)),
            Anchor::with_handles(Point::new(50.0, 0.0), Point::new(20.0, -40.0), Point::new(80.0, -40.0)),
            Anchor::with_handles(Point::new(100.0, 0.0), Point::new(100.0, -40.0), Point::new(100.0, 40.0)),
        ],
        true,
    );
    let grown = moved(&Displace { amplitude: 10.0, steps: 1, wander: 0.0, direction: DisplaceDirection::Normal, ..fractal() }, &flat_curvy);
    assert_eq!(grown.subpaths[0].anchors.len(), 6);
}
