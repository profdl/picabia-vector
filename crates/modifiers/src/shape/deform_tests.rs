// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, shapes};

use super::deform::{self, Field, Map};
use super::{GeometryOp, MAX_GEOMETRY_ANCHORS, RowAt, Shaper};
use crate::model::{Deform, DeformAxis, DeformMode, Falloff, ModifierKind, ShearOrigin};
use crate::{Output, evaluate, parse};

fn stack(rows: Value) -> crate::Stack {
    parse(&json!({"v": 1, "stack": rows}).to_string()).unwrap()
}

fn node(path: PathData) -> Node {
    Node::path(NodeId(1), path, Appearance::default_art())
}

fn rect(r: Rect) -> PathData {
    shapes::rectangle(r)
}

fn mode(m: DeformMode) -> Deform {
    Deform { mode: m, ..Deform::default() }
}

fn field(p: &Deform, b: Rect) -> Field {
    let paths = [rect(b)];
    match deform::map(p, b, deform::extent(&paths, b.center())).unwrap() {
        Map::Field { f, .. } => f,
        Map::Affine(_) => panic!("a field"),
    }
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-9
}

fn anchors(n: &Node) -> Vec<Point> {
    n.path_data().unwrap().subpaths.iter().flat_map(|s| s.anchors.iter().map(|a| a.p)).collect()
}

/// The object deformed by `p` as a source row (above every copy row).
fn source(path: PathData, p: Deform) -> Node {
    let out = Output {
        instances: vec![crate::Instance::identity()],
        geometry: vec![super::GeometryRow { modifier: "d".into(), op: GeometryOp::Deform(p), at: RowAt::Source }],
        ..Output::default()
    };
    let mut n = node(path);
    let mut sh = Shaper::new(&out);
    sh.source(&mut n);
    assert!(sh.notes.is_empty(), "{:?}", sh.notes);
    n
}

// ---- tlprocedural's point maps ----

/// Taper (`taperPolygon`): the scale across the axis runs from start to end through the falloff,
/// about the box's middle across.
#[test]
fn taper_scales_across_the_axis_from_start_to_end() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let f = field(&mode(DeformMode::Taper), b);
    assert!(near(f.at(Point::new(0.0, 0.0)), Point::new(0.0, 0.0)), "start scale 1");
    assert!(near(f.at(Point::new(100.0, 0.0)), Point::new(100.0, 20.0)), "end scale 0.2 about y = 25");
    assert!(near(f.at(Point::new(50.0, 50.0)), Point::new(50.0, 25.0 + 25.0 * 0.6)), "halfway: 0.6");
    let v = field(&Deform { axis: DeformAxis::Vertical, falloff: Falloff::EaseIn, ..mode(DeformMode::Taper) }, b);
    // t = 0.5 eased in: 0.25 of the way: scale 0.8 about x = 50.
    assert!(near(v.at(Point::new(0.0, 25.0)), Point::new(50.0 - 50.0 * 0.8, 25.0)));
    // The tapered rectangle's top edge is the straight line to (100, 20): every anchor on it.
    let n = source(rect(b), mode(DeformMode::Taper));
    let top: Vec<Point> = anchors(&n).into_iter().filter(|p| p.x > 1e-6 && p.x < 100.0 - 1e-6 && p.y < 25.0).collect();
    assert!(top.len() > 4, "the edge was split into pieces: {top:?}");
    for p in top {
        assert!((p.y - 0.2 * p.x).abs() < 1e-9, "{p:?}");
    }
}

/// Twist (`twistPolygon`): one end turns −angle/2 about the centre, the other +angle/2.
#[test]
fn twist_turns_the_ends_against_each_other() {
    let b = Rect::new(0.0, 0.0, 100.0, 20.0);
    let f = field(&mode(DeformMode::Twist), b);
    let c = Point::new(50.0, 10.0);
    assert!(near(f.at(c), c), "the middle stays");
    // The right end's middle turned +45° about the centre (y down: clockwise on screen).
    let r = f.at(Point::new(100.0, 10.0));
    let s = std::f64::consts::FRAC_1_SQRT_2;
    assert!(near(r, Point::new(50.0 + 50.0 * s, 10.0 + 50.0 * s)), "{r:?}");
    let l = f.at(Point::new(0.0, 10.0));
    assert!(near(l, Point::new(50.0 - 50.0 * s, 10.0 + 50.0 * s)), "{l:?}");
    // Smooth (tlprocedural's ease in-out) three quarters along: smoothstep(0.5) = 0.5 of 45°.
    let g = field(&Deform { falloff: Falloff::Smooth, ..mode(DeformMode::Twist) }, b);
    let q = g.at(Point::new(75.0, 10.0));
    let a = 22.5_f64.to_radians();
    assert!(near(q, Point::new(50.0 + 25.0 * a.cos(), 10.0 + 25.0 * a.sin())), "{q:?}");
}

/// Bend (`bendPolygon`): each point turns about the bend point by the angle times how far along
/// the axis it is from it.
#[test]
fn bend_turns_each_point_by_its_distance_from_the_bend_point() {
    let b = Rect::new(0.0, 0.0, 200.0, 20.0);
    let f = field(&mode(DeformMode::Bend), b);
    let pivot = Point::new(100.0, 10.0);
    assert!(near(f.at(pivot), pivot));
    // Both ends turn 22.5° (half of 45°) the same way about the pivot: they drop alike.
    let (l, r) = (f.at(Point::new(0.0, 10.0)), f.at(Point::new(200.0, 10.0)));
    let a = 22.5_f64.to_radians();
    assert!(near(r, Point::new(100.0 + 100.0 * a.cos(), 10.0 + 100.0 * a.sin())), "{r:?}");
    assert!(near(l, Point::new(100.0 - 100.0 * a.cos(), 10.0 + 100.0 * a.sin())), "{l:?}");
    // Vertical, bent at the top: the bottom swings by the whole angle.
    let v = field(&Deform { axis: DeformAxis::Vertical, bend_point: 0.0, ..mode(DeformMode::Bend) }, Rect::new(0.0, 0.0, 20.0, 200.0));
    let bottom = v.at(Point::new(10.0, 200.0));
    let a = 45_f64.to_radians();
    assert!(near(bottom, Point::new(10.0 + 200.0 * a.sin(), 200.0 * a.cos())), "{bottom:?}");
}

/// Shear (`shearPolygon`): affine about the origin: exact, curves and anchor count kept.
#[test]
fn shear_slants_about_its_origin_exactly() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let p = Deform { shear_x: 0.5, origin: ShearOrigin::TopLeft, ..mode(DeformMode::Shear) };
    let n = source(rect(b), p.clone());
    let pts = anchors(&n);
    assert_eq!(pts.len(), 4);
    assert!(pts.iter().any(|q| near(*q, Point::new(25.0, 50.0))), "{pts:?}");
    assert!(pts.iter().any(|q| near(*q, Point::new(0.0, 0.0))));
    // A circle stays four curves; centre origin: its middle doesn't move.
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0));
    let n = source(circle.clone(), Deform { shear_y: -1.0, ..mode(DeformMode::Shear) });
    assert_eq!(n.path_data().unwrap().anchor_count(), circle.anchor_count());
    let bb = n.path_data().unwrap().bounds().unwrap();
    assert!(near(bb.center(), Point::new(50.0, 50.0)), "{bb:?}");
}

/// Bulge (`bulgePolygon`): pushed out from the centre by strength × extent × ½ × falloff, nothing
/// past the radius; negative strength pinches.
#[test]
fn bulge_pushes_out_within_its_radius() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    let extent = 50.0 * std::f64::consts::SQRT_2;
    let f = field(&Deform { falloff: Falloff::Linear, ..mode(DeformMode::Bulge) }, b);
    let c = Point::new(50.0, 50.0);
    assert!(near(f.at(c), c), "the centre stays");
    let p = f.at(Point::new(75.0, 50.0));
    let push = 0.35 * extent * 0.5 * (1.0 - 25.0 / extent);
    assert!(near(p, Point::new(75.0 + push, 50.0)), "{p:?}");
    assert!(near(f.at(Point::new(100.0, 100.0)), Point::new(100.0, 100.0)), "the corner is at the radius");
    let small = field(&Deform { radius: 0.2, ..mode(DeformMode::Bulge) }, b);
    assert!(near(small.at(Point::new(75.0, 50.0)), Point::new(75.0, 50.0)), "past a 20% radius");
    let pinch = field(&Deform { strength: -1.0, ..mode(DeformMode::Bulge) }, b);
    assert!(pinch.at(Point::new(75.0, 50.0)).x < 75.0);
}

#[test]
fn falloffs_match_tlprocedurals_curves() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    for (falloff, at_quarter) in [(Falloff::Linear, 0.25), (Falloff::EaseIn, 0.0625), (Falloff::EaseOut, 0.4375), (Falloff::Smooth, 0.15625)] {
        let f = field(&Deform { falloff, start_scale: 1.0, end_scale: 0.0, ..mode(DeformMode::Taper) }, b);
        let y = f.at(Point::new(25.0, 0.0)).y;
        // scale = 1 − eased(¼): y = 25 − 25·scale.
        assert!((y - 25.0 * at_quarter).abs() < 1e-9, "{falloff:?}: {y}");
    }
}

// ---- Curves, the ramp, the frame ----

#[test]
fn curves_stay_curves() {
    let n = source(shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 50.0)), mode(DeformMode::Twist));
    let path = n.path_data().unwrap();
    assert!(path.anchor_count() > 4);
    assert!(path.subpaths[0].closed);
    assert!(path.subpaths[0].anchors.iter().all(|a| a.has_in() && a.has_out()), "smooth curves, not a polyline");
}

/// tlprocedural's `computeProgressiveScale`: nothing on the first copy, all of `amount` on the
/// last; reversed the other way; all of it with the ramp off, one copy, or on the object.
#[test]
fn the_ramp_runs_from_the_first_copy_to_the_last() {
    let p = Deform::default();
    assert_eq!(deform::ramp(&p, Some((0, 3))), 0.0);
    assert_eq!(deform::ramp(&p, Some((1, 3))), 0.5);
    assert_eq!(deform::ramp(&p, Some((2, 3))), 1.0);
    assert_eq!(deform::ramp(&Deform { progressive_reverse: true, ..p.clone() }, Some((0, 3))), 1.0);
    assert_eq!(deform::ramp(&Deform { progressive_amount: 0.5, ..p.clone() }, Some((2, 3))), 0.5);
    assert_eq!(deform::ramp(&Deform { progressive: false, ..p.clone() }, Some((0, 3))), 1.0);
    assert_eq!(deform::ramp(&p, Some((0, 1))), 1.0);
    assert_eq!(deform::ramp(&p, None), 1.0);
    // Taper's end scale ramps towards its start scale; the others scale.
    let half = deform::scaled(&p, 0.5);
    assert!((half.end_scale - 0.6).abs() < 1e-12);
    assert_eq!(half.angle, 45.0);
}

#[test]
fn below_an_array_each_copy_gets_its_share_and_the_first_is_untouched() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let out =
        evaluate(&stack(json!([{"id": "a", "kind": "linearArray", "count": 3, "offsetX": 150}, {"id": "d", "kind": "deform", "mode": "twist"}])), b);
    assert_eq!(out.instances.len(), 3);
    assert_eq!(out.geometry[0].at, RowAt::Copies);
    let (copies, notes) = super::shaped_copies(&node(rect(b)), &out);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(copies[0].path_data().unwrap().anchor_count(), 4, "the first copy: nothing to do, no anchors added");
    let h = |n: &Node| n.path_data().unwrap().bounds().unwrap().height();
    assert!(h(&copies[1]) > 50.0 + 1.0 && h(&copies[2]) > h(&copies[1]) + 1.0, "{} {}", h(&copies[1]), h(&copies[2]));
    // `copy` without an index: the whole effect, as on the object.
    let mut sh = Shaper::new(&out);
    let mut n = node(rect(b));
    sh.copy(&mut n);
    assert!(h(&n) > 51.0);
}

/// A copy turned by its transform is deformed along its own axis: the same shape as the object
/// deformed and then turned.
#[test]
fn a_turned_copy_is_deformed_in_its_own_frame() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let turn = Affine::rotate_about(90_f64.to_radians(), Point::new(200.0, 0.0));
    let out = Output {
        instances: vec![crate::Instance { transform: turn, ..crate::Instance::identity() }],
        geometry: vec![super::GeometryRow {
            modifier: "d".into(),
            op: GeometryOp::Deform(Deform { progressive: false, ..mode(DeformMode::Taper) }),
            at: RowAt::Copies,
        }],
        ..Output::default()
    };
    let mut copy = node(rect(b).transformed(turn));
    let mut sh = Shaper::new(&out);
    sh.copy_at(&mut copy, 0);
    let want = source(rect(b), Deform { progressive: false, ..mode(DeformMode::Taper) });
    let want: Vec<Point> = anchors(&want).into_iter().map(|p| turn * p).collect();
    let got = anchors(&copy);
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert!(g.distance(*w) < 1e-6, "{g:?} vs {w:?}");
    }
    assert_eq!(deform::frame_of(Affine::new([0.0, 0.0, 0.0, 0.0, 5.0, 5.0])), Affine::IDENTITY);
    assert_eq!(deform::frame_of(Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])), Affine::IDENTITY);
}

// ---- Caps, junk ----

#[test]
fn many_copies_share_the_anchor_cap() {
    let out = evaluate(
        &stack(
            json!([{"id": "a", "kind": "linearArray", "count": 1000, "offsetX": 1}, {"id": "d", "kind": "deform", "mode": "bulge", "progressive": false}]),
        ),
        Rect::new(0.0, 0.0, 100.0, 100.0),
    );
    let (copies, notes) = super::shaped_copies(&node(rect(Rect::new(0.0, 0.0, 100.0, 100.0))), &out);
    assert!(notes.is_empty(), "{notes:?}");
    let total: usize = copies.iter().map(|c| c.path_data().unwrap().anchor_count()).sum();
    assert!(total <= MAX_GEOMETRY_ANCHORS, "{total}");
    assert!(copies.iter().all(|c| c.path_data().unwrap().anchor_count() > 4), "still deformed, in coarser pieces");
    // A path already over its share keeps its anchors (each segment one piece).
    let big = SubPath::polyline(&(0..300).map(|i| Point::new(f64::from(i), f64::from(i % 7))).collect::<Vec<_>>(), false);
    let mut sh = Shaper::new(&out);
    let mut n = node(PathData::new(vec![big]));
    sh.copy_at(&mut n, 999);
    assert_eq!(n.path_data().unwrap().anchor_count(), 300);
}

#[test]
fn junk_params_clamp_to_tlprocedurals_ranges() {
    let p = Deform {
        start_scale: f64::NAN,
        end_scale: 99.0,
        angle: -1e9,
        bend_angle: f64::INFINITY,
        bend_point: 7.0,
        shear_x: -5.0,
        shear_y: f64::NEG_INFINITY,
        strength: 4.0,
        radius: 0.0,
        progressive_amount: -1.0,
        ..Deform::default()
    };
    let (op, warnings) = super::clamped(&ModifierKind::Deform(p)).unwrap();
    let GeometryOp::Deform(c) = op else { panic!() };
    assert_eq!(
        (c.start_scale, c.end_scale, c.angle, c.bend_angle, c.bend_point, c.shear_x, c.shear_y, c.strength, c.radius, c.progressive_amount),
        (1.0, 3.0, -720.0, 45.0, 1.0, -2.0, 0.0, 3.0, 0.01, 0.0)
    );
    assert!(warnings.len() >= 8, "{warnings:?}");
    // Unknown choices don't read: the row is kept as an unknown kind, not run.
    let s = stack(json!([{"id": "d", "kind": "deform", "mode": "melt"}]));
    assert!(matches!(s.modifiers[0].kind, ModifierKind::Other { .. }));
    assert!(evaluate(&s, Rect::new(0.0, 0.0, 1.0, 1.0)).geometry.is_empty());
}

#[test]
fn junk_geometry_never_panics_and_stays_finite() {
    let weird = [
        SubPath::new(vec![], false),
        SubPath::new(vec![], true),
        SubPath::polyline(&[Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::ZERO, Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::new(0.0, 100.0)], false),
        SubPath::polyline(&[Point::ZERO, Point::new(100.0, 0.0)], false),
        SubPath::polyline(&[Point::new(1e300, -1e300), Point::new(-1e300, 1e300)], false),
        SubPath::polyline(&[Point::new(1e-300, 0.0), Point::new(0.0, 1e-300), Point::new(2e-300, 1e-300)], true),
        SubPath::new(vec![Anchor::with_handles(Point::ZERO, Point::new(5.0, 5.0), Point::new(-5.0, 5.0)), Anchor::corner(Point::ZERO)], true),
        SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)], false),
        SubPath::polyline(&[Point::new(f64::INFINITY, 0.0), Point::new(1.0, 1.0)], false),
    ];
    let modes = [DeformMode::Taper, DeformMode::Twist, DeformMode::Bend, DeformMode::Shear, DeformMode::Bulge];
    let extremes = [
        Deform::default(),
        Deform {
            start_scale: 3.0,
            end_scale: 0.1,
            angle: 720.0,
            bend_angle: -360.0,
            bend_point: 0.0,
            shear_x: 2.0,
            shear_y: -2.0,
            strength: 3.0,
            radius: 0.01,
            ..Deform::default()
        },
        Deform {
            axis: DeformAxis::Vertical,
            falloff: Falloff::Smooth,
            strength: -3.0,
            radius: 2.0,
            origin: ShearOrigin::BottomRight,
            ..Deform::default()
        },
    ];
    for sp in &weird {
        for m in modes {
            for e in &extremes {
                let p = Deform { mode: m, ..e.clone() };
                let rows = vec![super::GeometryRow { modifier: "m".into(), op: GeometryOp::Deform(p.clone()), at: RowAt::Source }];
                let out = Output { instances: vec![crate::Instance::identity()], geometry: rows, ..Output::default() };
                let mut sh = Shaper::new(&out);
                let mut n = node(PathData::new(vec![sp.clone()]));
                sh.source(&mut n);
                let ok = n.path_data().unwrap().subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite() && a.h_in.is_finite());
                let was_ok = sp.anchors.iter().all(|a| a.p.is_finite());
                assert!(ok || !was_ok, "{p:?} on {sp:?}");
                // The bare path function too.
                let _ = super::shape_path(&PathData::new(vec![sp.clone()]), &GeometryOp::Deform(p));
            }
        }
    }
}

#[test]
fn guides_and_non_paths_are_left_alone_with_a_note() {
    let out = evaluate(&stack(json!([{"id": "d", "kind": "deform", "mode": "bend"}])), Rect::new(0.0, 0.0, 100.0, 100.0));
    let sym = Node { kind: NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY }, ..node(rect(Rect::new(0.0, 0.0, 10.0, 10.0))) };
    let mut guide = node(rect(Rect::new(0.0, 0.0, 100.0, 10.0)));
    if let NodeKind::Path { guide: g, .. } = &mut guide.kind {
        *g = true;
    }
    let mut g = Node::group(
        NodeId(2),
        vec![std::sync::Arc::new(node(rect(Rect::new(0.0, 0.0, 100.0, 20.0)))), std::sync::Arc::new(sym), std::sync::Arc::new(guide.clone())],
    );
    let mut sh = Shaper::new(&out);
    sh.source(&mut g);
    let kids = g.children().unwrap();
    assert!(kids[0].path_data().unwrap().anchor_count() > 4, "bent");
    assert_eq!(*kids[2], guide, "the guide as it was");
    assert_eq!(sh.notes.len(), 1, "{:?}", sh.notes);
    assert!(sh.notes[0].contains("Deform"));
}

#[test]
fn ruler_guides_are_left_alone_without_a_note() {
    let out = evaluate(&stack(json!([{"id": "d", "kind": "deform", "mode": "bend"}])), Rect::new(0.0, 0.0, 100.0, 100.0));
    let ruler = Node::new(NodeId(3), NodeKind::RulerGuide { vertical: true, pos: 50.0, artboard: None });
    let mut g = Node::group(NodeId(2), vec![std::sync::Arc::new(node(rect(Rect::new(0.0, 0.0, 100.0, 20.0)))), std::sync::Arc::new(ruler.clone())]);
    let mut sh = Shaper::new(&out);
    sh.source(&mut g);
    let kids = g.children().unwrap();
    assert!(kids[0].path_data().unwrap().anchor_count() > 4, "bent");
    assert_eq!(*kids[1], ruler, "the ruler guide as it was");
    assert!(sh.notes.is_empty(), "{:?}", sh.notes);
}

// ---- Mirrors and the ramp over array copies (PIC20.3a) ----

/// An asymmetric object: a 100 × 40 rectangle at (0, 0).
const OBJ: Rect = Rect::new(0.0, 0.0, 100.0, 40.0);

fn copies_of(rows: Value) -> Vec<Node> {
    let out = evaluate(&stack(rows), OBJ);
    let (copies, notes) = super::shaped_copies(&node(rect(OBJ)), &out);
    assert!(notes.is_empty(), "{notes:?}");
    copies
}

/// Whether `b` is `a` reflected by `m`: the same number of anchors, each of `m · a` on one of `b`.
fn mirrors(a: &Node, b: &Node, m: Affine) -> bool {
    let (pa, pb) = (anchors(a), anchors(b));
    pa.len() == pb.len() && pa.iter().all(|p| pb.iter().any(|q| q.distance(m * *p) < 1e-6))
}

/// `x' = 2·ax − x` and `y' = 2·ay − y` about the mirror lines through the box's centre (50, 20),
/// each moved 80 along its normal.
fn flips() -> (Affine, Affine) {
    (Affine::new([-1.0, 0.0, 0.0, 1.0, 2.0 * 130.0, 0.0]), Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * 100.0]))
}

/// Taper, Twist and Bend below a Mirror across both axes: every reflected copy is the mirror image
/// of the first (the frame keeps the reflection), and a lone mirror makes no ramp: each copy gets
/// the whole effect.
#[test]
fn a_mirrored_copy_is_the_mirror_image_of_the_deformed_object() {
    let (fx, fy) = flips();
    for mode in ["taper", "twist", "bend"] {
        let c = copies_of(json!([{"id": "m", "kind": "mirror", "axis": "both", "offset": 80}, {"id": "d", "kind": "deform", "mode": mode}]));
        assert_eq!(c.len(), 4);
        let whole = source(rect(OBJ), Deform { mode: serde_json::from_value(json!(mode)).unwrap(), ..Deform::default() });
        assert!(mirrors(&whole, &c[0], Affine::IDENTITY), "{mode}: the original gets the whole effect");
        for (k, m) in [(1, fx), (2, fy), (3, fy * fx)] {
            assert!(mirrors(&c[0], &c[k], m), "{mode}: copy {k} mirrors copy 0");
        }
    }
    // The reflection is in the frame: x mirrored, y mirrored.
    let close = |a: Affine, b: [f64; 6]| a.as_coeffs().iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12);
    assert!(close(deform::frame_of(Affine::new([-1.0, 0.0, 0.0, 1.0, 9.0, 0.0])), [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    assert!(close(deform::frame_of(Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 9.0])), [1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
}

/// tlprocedural's ramp steps over array copies: a mirror after an array repeats the ramp in its
/// reflected half; a mirror before an array is ramped as a pair.
#[test]
fn the_ramp_steps_over_array_copies_and_mirrors_share_it() {
    let pos = |rows: Value| {
        let out = evaluate(&stack(rows), OBJ);
        out.instances.iter().map(|i| out.ramp_position(i)).collect::<Vec<_>>()
    };
    let array = json!({"id": "a", "kind": "linearArray", "count": 3, "offsetX": 300});
    let mirror = json!({"id": "m", "kind": "mirror", "offset": 80});
    assert_eq!(pos(json!([mirror])), [None, None], "a lone mirror: no ramp");
    let three = |i| Some((i, 3));
    assert_eq!(pos(json!([array, mirror])), [three(0), three(1), three(2), three(0), three(1), three(2)]);
    assert_eq!(pos(json!([mirror, array])), [three(0), three(0), three(1), three(1), three(2), three(2)]);
    // Two arrays read as one combined array, the first counting fastest.
    let grid2 = json!({"id": "b", "kind": "linearArray", "count": 2, "offsetY": 300});
    assert_eq!(pos(json!([array, grid2]))[4], Some((4, 6)));
    // Array then mirror, deformed: the first copy and its reflection untouched, the last pair
    // fully twisted and mirror images of each other.
    let c = copies_of(json!([array, mirror, {"id": "d", "kind": "deform", "mode": "twist"}]));
    assert_eq!(c.len(), 6);
    assert_eq!((anchors(&c[0]).len(), anchors(&c[3]).len()), (4, 4));
    let (fx, _) = flips();
    // The reflection of copy 2 (moved 600 by the array) is about the same mirror line.
    assert!(mirrors(&c[2], &c[5], fx), "the last pair mirror each other");
    assert!(mirrors(&c[1], &c[4], fx));
    // Mirror then array: each mirrored pair shares its ramp.
    let c = copies_of(json!([mirror, array, {"id": "d", "kind": "deform", "mode": "twist"}]));
    assert_eq!((anchors(&c[0]).len(), anchors(&c[1]).len()), (4, 4));
    assert!(anchors(&c[2]).len() > 4 && anchors(&c[3]).len() > 4);
}

/// A copy whose transform isn't finite is skipped by whoever builds the copies; the ones after it
/// keep their own ramp (the index is the copy's place among all the copies).
#[test]
fn a_skipped_copy_leaves_the_ramp_aligned() {
    let p = Deform { mode: DeformMode::Taper, ..Deform::default() };
    let inst = |k: u32, t: Affine| crate::Instance { transform: t, index: k, stage_indices: vec![k], ..crate::Instance::identity() };
    let bad = Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let out = Output {
        instances: vec![inst(0, Affine::IDENTITY), inst(1, bad), inst(2, Affine::IDENTITY)],
        geometry: vec![super::GeometryRow { modifier: "d".into(), op: GeometryOp::Deform(p.clone()), at: RowAt::Copies }],
        array_stages: vec![crate::ArrayStage { array: true, copies: 3 }],
        ..Output::default()
    };
    let (copies, _) = super::shaped_copies(&node(rect(OBJ)), &out);
    assert_eq!(copies.len(), 2);
    assert_eq!(anchors(&copies[0]).len(), 4, "copy 0: nothing");
    let whole = source(rect(OBJ), p);
    assert!(mirrors(&whole, &copies[1], Affine::IDENTITY), "copy 2 (the second built): the whole taper, not half");
}
