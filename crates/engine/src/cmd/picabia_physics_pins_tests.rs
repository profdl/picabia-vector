//! PICABIA: pins (PIC31.8): Pin Points from points or Direct Selection anchors, Attach Points To,
//! Unpin, the canvas markers, and pinned cloth hanging in a session.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::pins::markers;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn shape(s: &mut Session, cmd: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(ex(s, cmd, json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap())
}

fn pins(s: &mut Session, id: NodeId) -> Value {
    ex(s, "physics.get", json!({"id": id.0}))["settings"]["soft"]["pins"].clone()
}

#[test]
fn pin_points_by_position_and_unpin_them() {
    let mut s = session();
    let cloth = shape(&mut s, "shape.rectangle", 100.0, 50.0, 200.0, 4.0);
    // Not soft yet: refused with the reason.
    assert!(s.execute("physics.pin", &json!({"id": cloth.0, "points": [[100, 52]]})).is_err());
    ex(&mut s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    let undo = s.doc().unwrap().history.undo.len();
    let out = ex(&mut s, "physics.pin", json!({"id": cloth.0, "points": [[100, 52], {"x": 300, "y": 52}]}));
    assert_eq!(out["pins"], json!([{"id": cloth.0, "count": 2}]));
    assert_eq!(pins(&mut s, cloth), json!([{"u": 0.0, "v": 0.5}, {"u": 1.0, "v": 0.5}]));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Pin Points");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Markers on the selected soft object, at rest.
    ex(&mut s, "select.set", json!({"ids": [cloth.0]}));
    let m = markers(&s);
    assert_eq!(m.len(), 2);
    assert!((m[0].0.x - 100.0).abs() < 1e-9 && (m[0].0.y - 52.0).abs() < 1e-9 && !m[0].1);
    // Unpin near a point, then all.
    ex(&mut s, "physics.unpin", json!({"id": cloth.0, "points": [[301, 53]]}));
    assert_eq!(pins(&mut s, cloth), json!([{"u": 0.0, "v": 0.5}]));
    ex(&mut s, "physics.unpin", json!({}));
    assert_eq!(pins(&mut s, cloth), Value::Null);
    // Junk.
    assert!(s.execute("physics.pin", &json!({"id": cloth.0, "points": [[f64::MAX, 0]]})).is_err());
    assert!(s.execute("physics.pin", &json!({"points": [[0, 0]]})).is_err());
    assert!(s.execute("physics.pin", &json!({"id": cloth.0, "points": "x"})).is_err());
    assert!(s.execute("physics.pin", &json!({})).is_err(), "nothing chosen");
}

#[test]
fn pin_the_anchors_chosen_with_direct_selection_and_attach_them() {
    let mut s = session();
    let cloth = shape(&mut s, "shape.rectangle", 100.0, 50.0, 200.0, 4.0);
    ex(&mut s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    let carrier = shape(&mut s, "shape.rectangle", 280.0, 20.0, 40.0, 20.0);
    // Direct Selection: the cloth's right-hand corners, and the carrier selected too.
    {
        let st = s.doc_mut().unwrap();
        st.selection.objects = vec![cloth, carrier];
        st.selection.anchors.insert(cloth, [(0, 1), (0, 2)].into_iter().collect());
    }
    // The carrier has no settings yet: refused.
    assert!(s.execute("physics.pin", &json!({"attachToSelected": true})).is_err());
    ex(&mut s, "physics.set", json!({"ids": [carrier.0], "props": {"bodyType": "dynamic"}}));
    {
        let st = s.doc_mut().unwrap();
        st.selection.objects = vec![cloth, carrier];
        st.selection.anchors.insert(cloth, [(0, 1), (0, 2)].into_iter().collect());
    }
    ex(&mut s, "physics.pin", json!({"attachToSelected": true}));
    let p = pins(&mut s, cloth);
    assert_eq!(p.as_array().unwrap().len(), 2, "{p}");
    assert!(p.as_array().unwrap().iter().all(|x| x["attachTo"] == carrier.0 && x["u"] == 1.0), "{p}");
    // Attached to a soft body: refused.
    assert!(s.execute("physics.pin", &json!({"id": cloth.0, "points": [[100, 50]], "attachTo": cloth.0})).is_err());
}

#[test]
fn pinned_cloth_hangs_in_a_session_and_the_markers_follow() {
    let mut s = session();
    let cloth = shape(&mut s, "shape.rectangle", 100.0, 50.0, 200.0, 4.0);
    ex(&mut s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    ex(&mut s, "physics.pin", json!({"id": cloth.0, "points": [[100, 52], [300, 52]]}));
    ex(&mut s, "select.set", json!({"ids": []}));
    ex(&mut s, "physics.session.open", json!({}));
    ex(&mut s, "physics.seek", json!({"step": 180}));
    let m = markers(&s);
    assert_eq!(m.len(), 2);
    for (q, attached) in &m {
        assert!(!attached && (q.y - 52.0).abs() < 1.0, "pinned in place: {q:?}");
    }
    // The middle sags between the pins.
    ex(&mut s, "physics.apply", json!({}));
    let b = s.doc().unwrap().doc.node(cloth).unwrap().geometric_bounds().unwrap();
    assert!(b.y0 < 55.0 && b.y1 > 80.0, "hangs between its pins: {b:?}");
}

// PIC31.8a (review).

#[test]
fn pins_go_on_the_body_the_simulation_takes() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 100.0, 50.0, 100.0, 40.0);
    let b = shape(&mut s, "shape.rectangle", 220.0, 50.0, 100.0, 40.0);
    ex(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(ex(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    // A soft path inside a rigid group: the group is the body, so the path can't be pinned.
    ex(&mut s, "physics.set", json!({"ids": [a.0], "props": {"bodyType": "soft"}}));
    ex(&mut s, "physics.set", json!({"ids": [g.0], "props": {"bodyType": "dynamic"}}));
    assert!(s.execute("physics.pin", &json!({"id": a.0, "points": [[100, 50]]})).is_err());
    // A soft group: pinning a member pins the group.
    ex(&mut s, "physics.set", json!({"ids": [g.0], "props": {"bodyType": "soft"}}));
    let out = ex(&mut s, "physics.pin", json!({"id": a.0, "points": [[100, 50]]}));
    assert_eq!(out["pins"][0]["id"], g.0, "{out}");
}

#[test]
fn over_the_cap_is_reported_and_nothing_new_makes_no_undo_step() {
    let mut s = session();
    let cloth = shape(&mut s, "shape.rectangle", 0.0, 0.0, 400.0, 4.0);
    ex(&mut s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    let pts: Vec<[f64; 2]> = (0..70).map(|i| [f64::from(i) * 5.0, 2.0]).collect();
    let out = ex(&mut s, "physics.pin", json!({"id": cloth.0, "points": pts}));
    assert_eq!(out["pins"][0]["count"], 64);
    assert!(out["warnings"][0].as_str().unwrap().contains("6 points left out"), "{out}");
    let undo = s.doc().unwrap().history.undo.len();
    ex(&mut s, "physics.pin", json!({"id": cloth.0, "points": [[0, 2]]}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "the same pin again: no undo step");
}

#[test]
fn attach_to_selected_takes_the_rigid_one() {
    let mut s = session();
    let cloth = shape(&mut s, "shape.rectangle", 100.0, 50.0, 200.0, 4.0);
    ex(&mut s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    let plain = shape(&mut s, "shape.rectangle", 10.0, 10.0, 20.0, 20.0);
    let carrier = shape(&mut s, "shape.rectangle", 280.0, 20.0, 40.0, 20.0);
    ex(&mut s, "physics.set", json!({"ids": [carrier.0], "props": {"bodyType": "static"}}));
    {
        let st = s.doc_mut().unwrap();
        st.selection.objects = vec![cloth, plain, carrier];
        st.selection.anchors.insert(cloth, [(0, 1)].into_iter().collect());
    }
    ex(&mut s, "physics.pin", json!({"attachToSelected": true}));
    assert_eq!(pins(&mut s, cloth)[0]["attachTo"], carrier.0);
    // Pinning the same point again without an object keeps the attachment.
    {
        let st = s.doc_mut().unwrap();
        st.selection.objects = vec![cloth];
        st.selection.anchors.insert(cloth, [(0, 1)].into_iter().collect());
    }
    ex(&mut s, "physics.pin", json!({}));
    assert_eq!(pins(&mut s, cloth)[0]["attachTo"], carrier.0);
}
