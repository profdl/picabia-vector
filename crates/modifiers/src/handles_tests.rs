// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};

use crate::handles::{Frame, HANDLE_GAP_PX, HandleKind, Motion, drag, guides, has_handles, label, spots};
use crate::model::{Modifier, ModifierKind};

fn square_node() -> Node {
    Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::default_art())
}

fn row(v: Value) -> Modifier {
    let mut o = v.as_object().unwrap().clone();
    o.insert("id".into(), json!("m1"));
    let stack = crate::parse(&json!({"v": 1, "stack": [o]}).to_string()).unwrap();
    stack.modifiers.first().unwrap().clone()
}

fn frame(n: &Node, zoom: f64) -> Frame<'_> {
    Frame { node: n, bounds: crate::node_bounds(n), zoom }
}

fn at(kind: &ModifierKind, f: &Frame, h: HandleKind) -> Point {
    spots(kind, f).into_iter().find(|s| s.kind == h).unwrap_or_else(|| panic!("{h:?} on {kind:?}")).at
}

/// `m` with the drag of `h` to `to` applied.
fn dragged(m: &Modifier, f: &Frame, h: HandleKind, to: Point, shift: bool) -> Modifier {
    let from = at(&m.kind, f, h);
    let props = drag(&m.kind, h, f, to, from, shift).unwrap_or_else(|| panic!("{h:?} to {to:?}"));
    let mut m = m.clone();
    m.set_props(&props).unwrap();
    m
}

fn near(a: Point, b: Point, tol: f64) -> bool {
    a.distance(b) <= tol
}

/// One row of every kind with handles, at its defaults and at other params.
fn rows() -> Vec<Value> {
    vec![
        json!({"kind": "linearArray"}),
        json!({"kind": "linearArray", "count": 5, "offsetX": 30, "offsetY": -40}),
        json!({"kind": "circularArray"}),
        json!({"kind": "circularArray", "radius": 80, "startAngle": 30, "sweep": 200, "centerX": 15, "centerY": -5}),
        json!({"kind": "gridArray"}),
        json!({"kind": "gridArray", "rows": 2, "cols": 4, "gapX": -10, "gapY": 35}),
        json!({"kind": "mirror", "axis": "both", "offset": 20}),
        json!({"kind": "wallpaper", "group": "p4m"}),
        json!({"kind": "wallpaper", "group": "pmg", "cellWidth": 120, "cellHeight": 80, "originX": 10, "originY": 5}),
        json!({"kind": "wallpaper", "group": "cmm", "cellWidth": 120, "cellHeight": 80}),
        json!({"kind": "wallpaper", "group": "p2", "cellWidth": 120, "cellHeight": 80, "angle": 70}),
        json!({"kind": "wallpaper", "group": "p1"}),
        json!({"kind": "deform", "mode": "taper"}),
        json!({"kind": "deform", "mode": "taper", "axis": "vertical", "startScale": 0.5, "endScale": 2}),
        json!({"kind": "deform", "mode": "twist", "angle": 60}),
        json!({"kind": "deform", "mode": "twist", "axis": "vertical", "angle": -100}),
        json!({"kind": "deform", "mode": "bend", "bendPoint": 0.3}),
        json!({"kind": "deform", "mode": "bend", "axis": "vertical"}),
        json!({"kind": "deform", "mode": "bulge", "radius": 0.7}),
        json!({"kind": "deform", "mode": "shear", "shearX": 0.4, "shearY": -0.2}),
        json!({"kind": "deform", "mode": "shear", "origin": "topLeft"}),
        json!({"kind": "deform", "mode": "shear", "origin": "bottomRight", "shearX": -0.5}),
        json!({"kind": "trim"}),
        json!({"kind": "trim", "start": 0.2, "end": 0.6, "offset": 0.3}),
        json!({"kind": "displace", "amplitude": 25}),
    ]
}

#[test]
fn a_drag_to_where_a_handle_is_keeps_its_params() {
    let n = square_node();
    for zoom in [0.25, 1.0, 3.0] {
        let f = frame(&n, zoom);
        for r in rows() {
            let m = row(r.clone());
            let all = spots(&m.kind, &f);
            assert!(!all.is_empty(), "{r} has handles");
            for s in all {
                let props = drag(&m.kind, s.kind, &f, s.at, s.at, false).unwrap_or_else(|| panic!("{r} {:?}", s.kind));
                let before = m.kind.params();
                for (k, v) in &props {
                    let (a, b) = (v.as_f64().unwrap(), before.get(k).and_then(Value::as_f64).unwrap());
                    assert!((a - b).abs() < 1e-6, "{r} {:?} {k}: {a} vs {b} at zoom {zoom}", s.kind);
                }
            }
        }
    }
}

#[test]
fn a_dragged_handle_lands_under_the_pointer() {
    let n = square_node();
    let f = frame(&n, 2.0);
    for r in rows() {
        let m = row(r.clone());
        for s in spots(&m.kind, &f) {
            // A move small enough to stay within every field's range.
            let to = s.at + Vec2::new(3.0, -2.0);
            let moved = dragged(&m, &f, s.kind, to, false);
            let now = at(&moved.kind, &f, s.kind);
            let want = match s.kind.motion(&m.kind) {
                Motion::Free => to,
                Motion::Along(d) => s.at + d * (to - s.at).dot(d),
                // Turned to the pointer's direction, at its own distance.
                Motion::Turn => {
                    let c = match (&moved.kind, s.kind) {
                        (ModifierKind::CircularArray(_), _) => spots(&moved.kind, &f).iter().find(|x| x.kind == HandleKind::RingCenter).unwrap().at,
                        _ => f.bounds.center(),
                    };
                    let d = now.distance(c);
                    c + (to - c) * (d / to.distance(c))
                }
            };
            if matches!(m.kind, ModifierKind::Trim(_)) {
                // Trim's knobs stay on the path: at the point of it nearest the pointer.
                assert!(now.distance(to) < to.distance(s.at) && now.distance(to) <= 3.0 + 1e-3, "{r} {:?}: {now:?} vs {to:?}", s.kind);
                continue;
            }
            assert!(near(now, want, 1e-6), "{r} {:?}: {now:?} vs {want:?}", s.kind);
        }
    }
}

#[test]
fn knobs_off_the_box_stay_a_fixed_distance_on_screen() {
    let n = square_node();
    for zoom in [0.5, 1.0, 4.0] {
        let f = frame(&n, zoom);
        let gap = HANDLE_GAP_PX / zoom;
        let m = row(json!({"kind": "displace", "amplitude": 0}));
        assert_eq!(at(&m.kind, &f, HandleKind::DisplaceAmplitude), Point::new(50.0, -gap), "{zoom}");
        let m = row(json!({"kind": "deform", "mode": "taper", "startScale": 1, "endScale": 1}));
        assert_eq!(at(&m.kind, &f, HandleKind::TaperStart), Point::new(-gap, 0.0), "{zoom}");
        assert_eq!(at(&m.kind, &f, HandleKind::TaperEnd), Point::new(100.0 + gap, 0.0), "{zoom}");
        // A displace knob dragged 10 pt up is 10 more amplitude, whatever the zoom.
        let m = row(json!({"kind": "displace", "amplitude": 5}));
        let k = at(&m.kind, &f, HandleKind::DisplaceAmplitude);
        let p = drag(&m.kind, HandleKind::DisplaceAmplitude, &f, k - Vec2::new(0.0, 10.0), k, false).unwrap();
        assert_eq!(p["amplitude"], json!(15.0));
    }
}

#[test]
fn the_handles_sit_where_the_copies_and_axes_are() {
    let n = square_node();
    let f = frame(&n, 1.0);
    // Pivot (50, 25).
    let m = row(json!({"kind": "linearArray", "count": 4, "offsetX": 100}));
    assert_eq!(at(&m.kind, &f, HandleKind::LinearEnd), Point::new(350.0, 25.0));
    let m = row(json!({"kind": "circularArray", "radius": 100}));
    assert_eq!(at(&m.kind, &f, HandleKind::RingCenter), Point::new(-50.0, 25.0));
    assert_eq!(at(&m.kind, &f, HandleKind::RingRadius), Point::new(-150.0, 25.0));
    let sweep = at(&m.kind, &f, HandleKind::RingSweep);
    assert!(near(sweep, Point::new(-50.0 + 100.0 + HANDLE_GAP_PX, 25.0), 1e-9), "{sweep:?}");
    let m = row(json!({"kind": "gridArray", "cols": 3, "rows": 2, "gapX": 10, "gapY": 20}));
    assert_eq!(at(&m.kind, &f, HandleKind::GridGapX), Point::new(220.0, 25.0));
    assert_eq!(at(&m.kind, &f, HandleKind::GridGapY), Point::new(50.0, 70.0));
    let m = row(json!({"kind": "mirror", "offset": 12}));
    assert_eq!(at(&m.kind, &f, HandleKind::MirrorVertical), Point::new(62.0, 25.0 - 150.0));
    let m = row(json!({"kind": "wallpaper", "group": "pmm", "cellWidth": 60, "cellHeight": 40, "originX": 5}));
    assert_eq!(at(&m.kind, &f, HandleKind::CellOrigin), Point::new(5.0, 0.0));
    assert_eq!(at(&m.kind, &f, HandleKind::CellA), Point::new(65.0, 0.0));
    assert_eq!(at(&m.kind, &f, HandleKind::CellB), Point::new(5.0, 40.0));
    let m = row(json!({"kind": "deform", "mode": "bend", "bendPoint": 0.25}));
    assert_eq!(at(&m.kind, &f, HandleKind::BendPoint), Point::new(25.0, 25.0));
    // A closed rectangle starts at its first corner and runs clockwise: 10% of 300 pt is 30 pt in.
    let m = row(json!({"kind": "trim", "start": 0.1, "end": 0.9}));
    let s = at(&m.kind, &f, HandleKind::TrimStart);
    assert!(near(s, Point::new(30.0, 0.0), 1e-6), "{s:?}");
}

#[test]
fn shift_constrains_the_drags() {
    let n = square_node();
    let f = frame(&n, 1.0);
    let m = row(json!({"kind": "linearArray", "count": 2, "offsetX": 100}));
    let end = at(&m.kind, &f, HandleKind::LinearEnd);
    let p = drag(&m.kind, HandleKind::LinearEnd, &f, Point::new(150.0, 120.0), end, true).unwrap();
    let (x, y) = (p["offsetX"].as_f64().unwrap(), p["offsetY"].as_f64().unwrap());
    assert!((x - y).abs() < 1e-9 && x > 0.0, "45°: {x}, {y}");
    let m = row(json!({"kind": "gridArray", "cols": 2, "rows": 2}));
    let k = at(&m.kind, &f, HandleKind::GridGapX);
    let p = drag(&m.kind, HandleKind::GridGapX, &f, k + Vec2::new(30.0, 0.0), k, true).unwrap();
    assert_eq!((p["gapX"].clone(), p["gapY"].clone()), (json!(130.0), json!(130.0)));
    let m = row(json!({"kind": "circularArray"}));
    let c = at(&m.kind, &f, HandleKind::RingCenter);
    let p = drag(&m.kind, HandleKind::RingCenter, &f, c + Vec2::new(40.0, 7.0), c, true).unwrap();
    assert_eq!((p["centerX"].clone(), p["centerY"].clone()), (json!(40.0), json!(0.0)));
    let k = at(&m.kind, &f, HandleKind::RingSweep);
    let r = 100.0 + HANDLE_GAP_PX;
    let p = drag(&m.kind, HandleKind::RingSweep, &f, c + Vec2::new(r * 0.2, -r), k, true).unwrap();
    assert_eq!(p["sweep"].as_f64().unwrap().rem_euclid(15.0), 0.0);
    // p2's second side: free, it sets the angle too; Shift keeps the angle.
    let m = row(json!({"kind": "wallpaper", "group": "p2", "cellHeight": 50, "angle": 90}));
    let k = at(&m.kind, &f, HandleKind::CellB);
    let p = drag(&m.kind, HandleKind::CellB, &f, k + Vec2::new(50.0, 0.0), k, false).unwrap();
    assert!((p["angle"].as_f64().unwrap() - 45.0).abs() < 1e-9 && (p["cellHeight"].as_f64().unwrap() - 50.0 * 2f64.sqrt()).abs() < 1e-9);
    let p = drag(&m.kind, HandleKind::CellB, &f, k + Vec2::new(50.0, 10.0), k, true).unwrap();
    assert!(p.get("angle").is_none() && (p["cellHeight"].as_f64().unwrap() - 60.0).abs() < 1e-9);
}

#[test]
fn junk_drags_set_nothing_or_stay_in_range() {
    let n = square_node();
    let f = frame(&n, 1.0);
    let nan = Point::new(f64::NAN, 0.0);
    let far = Point::new(1.0e300, -1.0e300);
    for r in rows() {
        let m = row(r.clone());
        for s in spots(&m.kind, &f) {
            assert!(drag(&m.kind, s.kind, &f, nan, s.at, false).is_none(), "{r}");
            assert!(drag(&m.kind, s.kind, &f, s.at, Point::new(f64::INFINITY, 0.0), false).is_none(), "{r}");
            for shift in [false, true] {
                if let Some(p) = drag(&m.kind, s.kind, &f, far, s.at, shift) {
                    let mut m2 = m.clone();
                    m2.set_props(&p).unwrap_or_else(|e| panic!("{r} {:?}: {e}", s.kind));
                    for (k, v) in &p {
                        let v = v.as_f64().unwrap();
                        let field = crate::number_field(m.kind.id(), k).unwrap();
                        assert!(v.is_finite() && v >= field.min && v <= field.max, "{r} {k} = {v}");
                    }
                }
            }
        }
        // A handle of another row sets nothing.
        assert!(drag(&m.kind, HandleKind::LinearEnd, &f, Point::ZERO, Point::ZERO, false).is_none() || m.kind.id() == "linearArray");
    }
    // Bad zooms and a non-finite box draw no knob that isn't finite.
    for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let f = frame(&n, zoom);
        for r in rows() {
            for s in spots(&row(r).kind, &f) {
                assert!(s.at.x.is_finite() && s.at.y.is_finite());
            }
        }
    }
    let bad = Frame { node: &n, bounds: Rect::new(0.0, 0.0, f64::NAN, 1.0), zoom: 1.0 };
    assert!(spots(&row(json!({"kind": "mirror"})).kind, &bad).is_empty());
}

#[test]
fn rows_without_handles_and_degenerate_objects_have_none() {
    let n = square_node();
    let f = frame(&n, 1.0);
    for k in ["transformEach", "randomize", "copyToShape", "resample", "tidy", "screen", "stencil"] {
        let m = row(json!({"kind": k}));
        assert!(spots(&m.kind, &f).is_empty() && !has_handles(&m.kind), "{k}");
        assert!(guides(&m.kind, &f).is_empty() && label(&m.kind, HandleKind::LinearEnd).is_none(), "{k}");
    }
    assert!(spots(&row(json!({"kind": "linearArray", "count": 1})).kind, &f).is_empty());
    assert!(spots(&row(json!({"kind": "gridArray", "rows": 1, "cols": 1})).kind, &f).is_empty());
    // Square lattices have no second side; p1 has no origin.
    let kinds: Vec<HandleKind> = spots(&row(json!({"kind": "wallpaper", "group": "p4m"})).kind, &f).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [HandleKind::CellA, HandleKind::CellOrigin]);
    let kinds: Vec<HandleKind> = spots(&row(json!({"kind": "wallpaper", "group": "p1"})).kind, &f).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [HandleKind::CellA, HandleKind::CellB]);
    // A point has no box to deform and no length to trim.
    let dot = Node::path(NodeId(2), shapes::rectangle(Rect::new(5.0, 5.0, 5.0, 5.0)), Appearance::default_art());
    let fd = frame(&dot, 1.0);
    for r in [json!({"kind": "deform", "mode": "bend"}), json!({"kind": "deform", "mode": "taper"}), json!({"kind": "trim"})] {
        assert!(spots(&row(r.clone()).kind, &fd).is_empty(), "{r}");
    }
}

#[test]
fn labels_read_the_values() {
    let m = row(json!({"kind": "linearArray", "offsetX": 120.04, "offsetY": -0.01}));
    assert_eq!(label(&m.kind, HandleKind::LinearEnd).as_deref(), Some("Offset 120, 0"));
    let m = row(json!({"kind": "circularArray", "sweep": 270}));
    assert_eq!(label(&m.kind, HandleKind::RingSweep).as_deref(), Some("Sweep 270°"));
    let m = row(json!({"kind": "deform", "mode": "bend", "bendPoint": 0.25}));
    assert_eq!(label(&m.kind, HandleKind::BendPoint).as_deref(), Some("Bend point 25%"));
    let m = row(json!({"kind": "wallpaper", "group": "p2", "cellHeight": 80, "angle": 60}));
    assert_eq!(label(&m.kind, HandleKind::CellB).as_deref(), Some("Cell height 80, 60°"));
    assert_eq!(HandleKind::CellB.fields(&m.kind), ["cellHeight", "angle"]);
}
