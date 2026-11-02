//! PICABIA: the canvas handles of modifier rows: which rows show them, dragging one as one undo
//! step, Escape, bound fields, zoom and a turned view, junk.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind, ScreenFrame, ToolKey};

use super::tests::{arrayed, ex, rect, session, undo_labels};
use crate::{Session, UiRequest, ViewInfo};

fn view(zoom: f64) -> ViewInfo {
    ViewInfo { zoom, ..ViewInfo::default() }
}

/// A view turned 90° clockwise at `zoom`.
fn turned(zoom: f64) -> ViewInfo {
    let screen = ScreenFrame { origin: Point::ZERO, right: Vec2::new(0.0, -1.0 / zoom), down: Vec2::new(1.0 / zoom, 0.0), size: (800.0, 600.0) };
    ViewInfo { zoom, screen: Some(screen), ..ViewInfo::default() }
}

fn ptr(s: &mut Session, kind: PointerKind, p: Point, v: ViewInfo) -> Vec<UiRequest> {
    s.pointer(&PointerEvent::new(kind, p.x, p.y), v).unwrap()
}

/// Press at `from`, drag through `to` and release there.
fn drag(s: &mut Session, from: Point, to: Point, v: ViewInfo) {
    ptr(s, PointerKind::Move, from, v);
    ptr(s, PointerKind::Down, from, v);
    ptr(s, PointerKind::Drag, from.lerp(to, 0.5), v);
    ptr(s, PointerKind::Drag, to, v);
    ptr(s, PointerKind::Up, to, v);
}

fn handles(s: &mut Session) -> Vec<Value> {
    ex(s, "modifier.handles", json!({}))["handles"].as_array().unwrap().clone()
}

fn handle_at(s: &mut Session, name: &str) -> Point {
    let h = handles(s).into_iter().find(|h| h["handle"] == name).unwrap_or_else(|| panic!("no {name}"));
    Point::new(h["x"].as_f64().unwrap(), h["y"].as_f64().unwrap())
}

fn param(s: &mut Session, id: NodeId, row: usize, key: &str) -> f64 {
    ex(s, "modifier.list", json!({"id": id.0}))["modifiers"][row][key].as_f64().unwrap()
}

/// The rectangle at 0,0 (100 × 100) with a 5-copy linear array 120 apart, selected, with the
/// Selection tool.
fn linear() -> (Session, NodeId) {
    let (mut s, r) = arrayed();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, [r]);
    (s, r)
}

#[test]
fn dragging_the_linear_end_sets_the_offset_in_one_undo_step() {
    let (mut s, r) = linear();
    let v = view(1.0);
    let end = handle_at(&mut s, "linearEnd");
    assert_eq!(end, Point::new(530.0, 50.0), "the last copy's centre");
    let steps = undo_labels(&s).len();
    // Grabbed 3 pt off its centre: the handle keeps the grab.
    drag(&mut s, end + Vec2::new(3.0, 0.0), Point::new(433.0, 90.0), v);
    assert_eq!((param(&mut s, r, 0, "offsetX"), param(&mut s, r, 0, "offsetY")), (95.0, 10.0));
    assert_eq!(undo_labels(&s).len(), steps + 1, "one step");
    assert_eq!(undo_labels(&s).last().unwrap(), "Set Modifier");
    assert_eq!(s.doc().unwrap().selection.objects, [r], "the object stays selected and unmoved");
    assert_eq!(s.doc().unwrap().doc.node(r).unwrap().geometric_bounds().unwrap().x0, 0.0);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(param(&mut s, r, 0, "offsetX"), 120.0);
    // A drag that comes back where it began leaves no step.
    let steps = undo_labels(&s).len();
    ptr(&mut s, PointerKind::Down, end, v);
    ptr(&mut s, PointerKind::Drag, end + Vec2::new(40.0, 0.0), v);
    ptr(&mut s, PointerKind::Drag, end, v);
    ptr(&mut s, PointerKind::Up, end, v);
    assert_eq!(undo_labels(&s).len(), steps);
    assert!(!s.tool_busy());
}

#[test]
fn escape_puts_the_params_back() {
    let (mut s, r) = linear();
    let v = view(1.0);
    let end = handle_at(&mut s, "linearEnd");
    let steps = undo_labels(&s).len();
    ptr(&mut s, PointerKind::Down, end, v);
    ptr(&mut s, PointerKind::Drag, end + Vec2::new(-200.0, 50.0), v);
    assert_ne!(param(&mut s, r, 0, "offsetX"), 120.0, "previewed");
    assert!(s.tool_busy() && s.tool_claims_key(ToolKey::Escape, v));
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!((param(&mut s, r, 0, "offsetX"), param(&mut s, r, 0, "offsetY")), (120.0, 0.0));
    assert_eq!(undo_labels(&s).len(), steps);
    assert!(!s.tool_busy() && !s.tool_claims_key(ToolKey::Escape, v));
    // The release after it changes nothing and starts nothing.
    ptr(&mut s, PointerKind::Up, end + Vec2::new(-200.0, 50.0), v);
    assert_eq!(param(&mut s, r, 0, "offsetX"), 120.0);
    assert_eq!(s.doc().unwrap().selection.objects, [r]);
}

#[test]
fn zoom_and_a_turned_view_reach_and_drag_in_document_points() {
    for v in [view(4.0), turned(4.0), turned(0.5)] {
        let (mut s, r) = linear();
        let end = handle_at(&mut s, "linearEnd");
        // 7 screen pixels reach: 1.75 pt at 4×, 14 pt at 0.5×.
        let reach = super::HIT_PX / v.zoom;
        assert_eq!(s.cursor(end + Vec2::new(0.0, reach * 0.9), Mods::default(), v), Cursor::Move);
        ptr(&mut s, PointerKind::Move, end + Vec2::new(0.0, reach * 1.5), v);
        assert!(s.picabia_mod_handles(v).unwrap().handles.iter().all(|h| !h.hot), "out of reach");
        ptr(&mut s, PointerKind::Move, end + Vec2::new(0.0, reach * 0.9), v);
        assert!(s.picabia_mod_handles(v).unwrap().handles.iter().any(|h| h.hot), "in reach");
        drag(&mut s, end + Vec2::new(0.0, reach * 0.9), Point::new(250.0, 50.0 + reach * 0.9 + 40.0), v);
        assert_eq!(param(&mut s, r, 0, "offsetX"), 50.0, "{v:?}");
        assert!((param(&mut s, r, 0, "offsetY") - 10.0).abs() < 1e-9, "{v:?}");
    }
    // A mirror's knob moves along x: a sideways resize cursor, turned with the view.
    let (mut s, r) = linear();
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror"}));
    let k = handle_at(&mut s, "mirrorVertical");
    assert_eq!(s.cursor(k, Mods::default(), view(1.0)), Cursor::ResizeH);
    assert_eq!(s.cursor(k, Mods::default(), turned(1.0)), Cursor::ResizeV);
    // Alt, Cmd and Ctrl leave the press to the tool.
    assert_ne!(s.cursor(k, Mods { alt: true, ..Mods::default() }, view(1.0)), Cursor::ResizeH);
}

#[test]
fn the_open_rows_show_their_handles_else_the_last_row_with_handles() {
    let (mut s, r) = linear();
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "transformEach"}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror"}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "randomize"}));
    let names = |s: &mut Session| handles(s).iter().map(|h| h["handle"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(&mut s), ["mirrorVertical"], "the last row with handles");
    s.picabia_set_handle_rows(r, vec!["m1".into()]);
    assert_eq!(names(&mut s), ["linearEnd"]);
    s.picabia_set_handle_rows(r, vec!["m1".into(), "m3".into(), "m2".into()]);
    assert_eq!(names(&mut s), ["linearEnd", "mirrorVertical"], "transform each has none");
    // Rows of another object, or rows without handles: the last row again.
    s.picabia_set_handle_rows(NodeId(r.0 + 999), vec!["m1".into()]);
    assert_eq!(names(&mut s), ["mirrorVertical"]);
    s.picabia_set_handle_rows(r, vec!["m2".into()]);
    assert_eq!(names(&mut s), ["mirrorVertical"]);
    // A turned-off row shows none.
    ex(&mut s, "modifier.toggle", json!({"ids": [r.0], "modifier": "m3", "enabled": false}));
    assert_eq!(names(&mut s), ["linearEnd"]);
}

#[test]
fn no_handles_without_one_editable_object_with_the_selection_tool() {
    let (mut s, r) = linear();
    assert_eq!(handles(&mut s).len(), 1);
    s.select_tool("directSelection", ViewInfo::default()).unwrap();
    assert!(handles(&mut s).is_empty(), "another tool");
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let other = rect(&mut s, 800.0);
    ex(&mut s, "select.all", json!({}));
    assert!(s.doc().unwrap().selection.objects.contains(&other));
    assert!(handles(&mut s).is_empty(), "two objects");
    ex(&mut s, "select.none", json!({}));
    assert!(handles(&mut s).is_empty(), "nothing");
    ex(&mut s, "object.lock", json!({"ids": [r.0]}));
    let _ = s.execute("select.set", &json!({"ids": [r.0]}));
    assert!(handles(&mut s).is_empty(), "locked");
    // A plain object.
    let mut s = session();
    rect(&mut s, 0.0);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    assert!(handles(&mut s).is_empty());
}

#[test]
fn a_bound_field_draws_its_handle_greyed_and_doesnt_drag() {
    let (mut s, r) = linear();
    ex(&mut s, "param.add", json!({"name": "step", "value": 60, "min": 0, "max": 500}));
    ex(&mut s, "modifier.bind", json!({"ids": [r.0], "index": 0, "field": "offsetX", "binding": {"param": "step"}}));
    let h = handles(&mut s).into_iter().next().unwrap();
    assert_eq!(h["bound"], true);
    // Placed by the bound value: 4 × 60 from the pivot.
    let end = Point::new(h["x"].as_f64().unwrap(), h["y"].as_f64().unwrap());
    assert_eq!(end, Point::new(290.0, 50.0));
    let v = view(1.0);
    assert_eq!(s.cursor(end, Mods::default(), v), Cursor::NotAllowed);
    let steps = undo_labels(&s).len();
    let said = ptr(&mut s, PointerKind::Down, end, v);
    assert!(said.iter().any(|r| matches!(r, UiRequest::Status(m) if m.contains("bound"))), "{said:?}");
    ptr(&mut s, PointerKind::Drag, end + Vec2::new(80.0, 30.0), v);
    ptr(&mut s, PointerKind::Up, end + Vec2::new(80.0, 30.0), v);
    assert_eq!((param(&mut s, r, 0, "offsetX"), param(&mut s, r, 0, "offsetY")), (120.0, 0.0), "the stored values stay");
    assert_eq!(undo_labels(&s).len(), steps);
    let view = s.picabia_mod_handles(v).unwrap();
    assert!(view.readout.is_some_and(|(_, t)| t.contains("Bound")), "the hint shows");
}

#[test]
fn circular_mirror_and_deform_handles_drag() {
    let (mut s, r) = linear();
    ex(&mut s, "modifier.remove", json!({"ids": [r.0], "index": 0}));
    let v = view(1.0);
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "circularArray", "props": {"radius": 100}}));
    // Centre at the pivot less the radius: (-50, 50); the radius knob opposite the original.
    assert_eq!(handle_at(&mut s, "ringCenter"), Point::new(-50.0, 50.0));
    let k = handle_at(&mut s, "ringRadius");
    drag(&mut s, k, k - Vec2::new(60.0, 0.0), v);
    assert_eq!(param(&mut s, r, 0, "radius"), 130.0);
    let c = handle_at(&mut s, "ringCenter");
    drag(&mut s, c, c + Vec2::new(10.0, -20.0), v);
    assert_eq!((param(&mut s, r, 0, "centerX"), param(&mut s, r, 0, "centerY")), (10.0, -20.0));
    let c = handle_at(&mut s, "ringCenter");
    let k = handle_at(&mut s, "ringSweep");
    // Up from the centre: the sweep ends at −90° (from 360, the nearest way round).
    drag(&mut s, k, c + Vec2::new(0.0, -200.0), v);
    assert!((param(&mut s, r, 0, "sweep") - 270.0).abs() < 1e-9);
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror", "props": {"axis": "horizontal"}}));
    let k = handle_at(&mut s, "mirrorHorizontal");
    drag(&mut s, k, k + Vec2::new(30.0, 25.0), v);
    assert_eq!(param(&mut s, r, 1, "offset"), 25.0);
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "deform", "props": {"mode": "bend", "bendPoint": 0.5}}));
    let k = handle_at(&mut s, "bendPoint");
    assert_eq!(k, Point::new(50.0, 50.0));
    drag(&mut s, k + Vec2::new(0.0, 2.0), Point::new(20.0, 52.0), v);
    assert!((param(&mut s, r, 2, "bendPoint") - 0.2).abs() < 1e-9);
    let labels = undo_labels(&s);
    assert_eq!(labels.iter().filter(|l| *l == "Set Modifier").count(), 5, "one step a drag: {labels:?}");
}

#[test]
fn junk_drags_stay_finite_and_in_range() {
    let (mut s, r) = linear();
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "circularArray"}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "deform", "props": {"mode": "twist"}}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "trim"}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "displace"}));
    s.picabia_set_handle_rows(r, ["m1", "m2", "m3", "m4", "m5"].map(String::from).to_vec());
    let all = handles(&mut s);
    assert_eq!(all.len(), 1 + 3 + 1 + 2 + 1, "{all:?}");
    for zoom in [1.0, 1.0e-9, 1.0e9, f64::NAN] {
        let v = view(zoom);
        for h in handles(&mut s) {
            let at = Point::new(h["x"].as_f64().unwrap(), h["y"].as_f64().unwrap());
            for to in [Point::new(f64::NAN, 1.0), Point::new(1.0e300, -1.0e300), Point::new(f64::INFINITY, 0.0), at + Vec2::new(-1.0e7, 3.0e7)] {
                ptr(&mut s, PointerKind::Down, at, v);
                let _ = s.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), v);
                let _ = s.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y), v);
                assert!(!s.tool_busy());
            }
        }
    }
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    for m in l["modifiers"].as_array().unwrap() {
        for (k, v) in m.as_object().unwrap() {
            if let Some(x) = v.as_f64() {
                assert!(x.is_finite(), "{k} = {x}");
                if let Some(f) = vectorcraft_modifiers::number_field(m["kind"].as_str().unwrap(), k) {
                    assert!(x >= f.min && x <= f.max, "{k} = {x}");
                }
            }
        }
    }
    assert!(l["warnings"].as_array().is_some());
}
