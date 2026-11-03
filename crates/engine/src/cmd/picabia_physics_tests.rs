//! PICABIA: the physics commands: settings on objects (kept through save and reopen and undo),
//! and Simulate dropping objects onto the artboard's floor as one undo step.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Rect;

use crate::{EngineError, Session};

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

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn board(s: &Session) -> Rect {
    s.doc().unwrap().doc.artboards[0].rect
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

#[test]
fn set_get_and_clear_with_undo() {
    let mut s = session();
    let r = shape(&mut s, "shape.rectangle", 10.0, 10.0, 50.0, 40.0);
    assert_eq!(ex(&mut s, "physics.get", json!({"id": r.0}))["settings"], Value::Null);
    let out = ex(&mut s, "physics.set", json!({"ids": [r.0], "props": {"bodyType": "static", "restitution": 3, "wobble": 1, "friction": "lots"}}));
    assert_eq!(out["settings"]["bodyType"], "static");
    assert_eq!(out["settings"]["restitution"], 1.0);
    assert_eq!(out["settings"]["gravityScale"], 1.2, "defaults fill the rest");
    assert_eq!(out["warnings"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(undo_labels(&s).last().unwrap(), "Physics Settings");
    let got = ex(&mut s, "physics.get", json!({"id": r.0}));
    assert_eq!(got["settings"]["friction"], 0.4);
    // Setting the same again: no undo step.
    let n = undo_labels(&s).len();
    ex(&mut s, "physics.set", json!({"ids": [r.0], "props": {"bodyType": "static"}}));
    assert_eq!(undo_labels(&s).len(), n);
    ex(&mut s, "physics.clear", json!({"ids": [r.0]}));
    assert_eq!(ex(&mut s, "physics.get", json!({"id": r.0}))["settings"], Value::Null);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "physics.get", json!({"id": r.0}))["settings"]["bodyType"], "static");
    // Layers are refused.
    let layer = s.doc().unwrap().doc.layers[0].id;
    assert!(s.execute("physics.set", &json!({"ids": [layer.0]})).is_err());
}

#[test]
fn simulate_drops_the_selection_to_the_floor_and_undo_puts_it_back() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 40.0, 20.0, 60.0, 40.0);
    let b = shape(&mut s, "shape.ellipse", 200.0, 30.0, 50.0, 50.0);
    ex(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let before = s.doc().unwrap().doc.clone();
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["bodies"], 2, "{out}");
    assert_eq!(out["moved"].as_array().unwrap().len(), 2, "{out}");
    assert_eq!(out["settled"], true, "{out}");
    assert_eq!(out["artboard"], 0);
    let floor = board(&s).y1;
    for id in [a, b] {
        let r = bounds(&s, id);
        assert!((r.y1 - floor).abs() < 2.0, "{id:?} rests at {}", r.y1);
    }
    assert_eq!(undo_labels(&s).last().unwrap(), "Simulate Physics");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc, before, "undo restores exactly");
}

#[test]
fn static_settings_hold_dynamic_ones_and_locked_objects_stay() {
    let mut s = session();
    let shelf = shape(&mut s, "shape.rectangle", 100.0, 150.0, 200.0, 20.0);
    let falling = shape(&mut s, "shape.rectangle", 170.0, 40.0, 60.0, 40.0);
    let locked = shape(&mut s, "shape.rectangle", 20.0, 20.0, 30.0, 30.0);
    ex(&mut s, "physics.set", json!({"ids": [shelf.0], "props": {"bodyType": "static"}}));
    ex(&mut s, "physics.set", json!({"ids": [falling.0, locked.0]}));
    ex(&mut s, "select.set", json!({"ids": [locked.0]}));
    ex(&mut s, "object.lock", json!({}));
    ex(&mut s, "select.set", json!({"ids": []}));
    let still = bounds(&s, shelf);
    let lock_at = bounds(&s, locked);
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["moved"], json!([falling.0]), "{out}");
    assert_eq!(bounds(&s, shelf), still);
    assert_eq!(bounds(&s, locked), lock_at);
    assert!((bounds(&s, falling).y1 - still.y0).abs() < 2.0, "{:?}", bounds(&s, falling));
}

#[test]
fn a_group_with_settings_falls_as_one_and_its_members_move_together() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 100.0, 20.0, 40.0, 40.0);
    let b = shape(&mut s, "shape.rectangle", 160.0, 20.0, 40.0, 40.0);
    ex(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(ex(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    ex(&mut s, "physics.set", json!({"ids": [g.0]}));
    ex(&mut s, "select.set", json!({"ids": []}));
    let gap = bounds(&s, b).x0 - bounds(&s, a).x1;
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["moved"], json!([g.0]), "{out}");
    let (ra, rb) = (bounds(&s, a), bounds(&s, b));
    assert!((rb.x0 - ra.x1 - gap).abs() < 0.5 && (ra.y1 - rb.y1).abs() < 0.5, "{ra:?} {rb:?}");
    assert!((ra.y1 - board(&s).y1).abs() < 2.0);
}

#[test]
fn walls_off_with_sideways_gravity_and_bad_params() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 100.0, 100.0, 40.0, 40.0);
    ex(&mut s, "select.set", json!({"ids": [a.0]}));
    // Without walls nothing stops it: it falls out of the world and is left where it was.
    let out = ex(&mut s, "physics.simulate", json!({"walls": false, "gravityY": 100000, "steps": 20000}));
    assert_eq!(out["lost"], json!([a.0]), "{out}");
    assert_eq!(out["artboard"], Value::Null);
    let out = ex(&mut s, "physics.simulate", json!({"gravityX": 981, "gravityY": 0}));
    assert_eq!(out["moved"], json!([a.0]));
    assert!((bounds(&s, a).x1 - board(&s).x1).abs() < 2.0);
    for p in
        [json!({"steps": 0}), json!({"steps": 1.5}), json!({"steps": "9"}), json!({"gravityX": 1e9}), json!({"artboard": 7}), json!({"walls": 1})]
    {
        assert!(s.execute("physics.simulate", &p).is_err(), "{p}");
    }
    // A few steps only: it moves a little and isn't settled.
    let out = ex(&mut s, "physics.simulate", json!({"steps": 2, "gravityX": -981, "gravityY": 0}));
    assert_eq!((out["steps"].as_u64(), out["settled"].as_bool()), (Some(2), Some(false)));
}

#[test]
fn nothing_to_drop_is_refused_without_an_undo_step() {
    let mut s = session();
    shape(&mut s, "shape.rectangle", 100.0, 100.0, 40.0, 40.0);
    ex(&mut s, "select.set", json!({"ids": []}));
    let n = undo_labels(&s).len();
    assert!(matches!(s.execute("physics.simulate", &json!({})), Err(EngineError::Disabled(..))));
    assert_eq!(undo_labels(&s).len(), n);
}

#[test]
fn damaged_settings_are_skipped_and_reported() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 100.0, 20.0, 40.0, 40.0);
    let b = shape(&mut s, "shape.rectangle", 200.0, 20.0, 40.0, 40.0);
    ex(&mut s, "physics.set", json!({"ids": [a.0, b.0]}));
    s.edit("test", |d, _| {
        let n = d.node_mut(b).unwrap();
        n.edit_attrs(|at| at.foreign.iter_mut().for_each(|f| f.value = "{not json".into()));
        Ok(())
    })
    .unwrap();
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["moved"], json!([a.0]));
    assert_eq!(out["skipped"][0]["id"], b.0, "{out}");
    let got = ex(&mut s, "physics.get", json!({"id": b.0}));
    assert!(got["error"].is_string());
}

#[test]
fn settings_survive_save_and_reopen() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 100.0, 20.0, 40.0, 40.0);
    ex(&mut s, "physics.set", json!({"ids": [a.0], "props": {"bodyType": "kinematic", "colliderType": "box", "mass": 5}}));
    let before = ex(&mut s, "physics.get", json!({"id": a.0}))["settings"].clone();
    let saved = ex(&mut s, "document.serialize", json!({"format": "vectorcraft"}));
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "physics.vectorcraft", "dataBase64": saved["dataBase64"]}));
    assert_eq!(ex(&mut t, "physics.get", json!({"id": a.0}))["settings"], before);
}

#[test]
fn the_same_scene_simulates_to_the_same_document() {
    let run = || {
        let mut s = session();
        let mut ids = vec![];
        for i in 0..6 {
            let x = 30.0 + f64::from(i) * 55.0;
            let cmd = if i % 2 == 0 { "shape.rectangle" } else { "shape.ellipse" };
            ids.push(shape(&mut s, cmd, x, 10.0 + f64::from(i % 3) * 30.0, 40.0, 30.0).0);
        }
        ex(&mut s, "select.set", json!({"ids": ids}));
        ex(&mut s, "physics.simulate", json!({}));
        vectorcraft_format::save(&s.doc().unwrap().doc, false)
    };
    assert_eq!(run(), run());
}

/// Type, a placed image and a live shape land on the floor, turned and moved as Rotate and Move
/// would move them: still type, an image and a live shape afterwards.
#[test]
fn type_an_image_and_a_live_shape_are_moved_as_the_transform_commands_move_them() {
    use vectorcraft_doc::NodeKind;
    let mut s = session();
    let text = NodeId(ex(&mut s, "text.create", json!({"x": 40, "y": 40, "size": 24, "text": "Drop"}))["id"].as_u64().unwrap());
    let mut png = vec![];
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let placed = ex(&mut s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [200, 60]}));
    let img = NodeId(placed["ids"][0].as_u64().unwrap());
    let live = shape(&mut s, "shape.rectangle", 300.0, 30.0, 50.0, 30.0);
    ex(&mut s, "select.set", json!({"ids": [text.0, img.0, live.0]}));
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["moved"].as_array().unwrap().len(), 3, "{out}");
    let floor = board(&s).y1;
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(text).unwrap().kind, NodeKind::Text(_)));
    assert!(matches!(doc.node(img).unwrap().kind, NodeKind::Image(_)));
    assert!(matches!(&doc.node(live).unwrap().kind, NodeKind::Path { live: Some(_), .. }), "still a live rectangle");
    for id in [text, img, live] {
        let r = doc.node(id).unwrap().geometric_bounds().unwrap();
        assert!((r.y1 - floor).abs() < 3.0, "{id:?} rests at {} (floor {floor})", r.y1);
    }
}

/// A live rectangle that comes to rest turned keeps its turn: its own angle is the body's.
#[test]
fn a_turned_live_rectangle_keeps_its_angle() {
    let mut s = session();
    // A static slope from (0, 200) to (400, 280).
    let slope = NodeId(ex(&mut s, "path.create", json!({"d": "M0 200 L400 280 L400 300 L0 300 Z"}))["id"].as_u64().unwrap());
    ex(&mut s, "physics.set", json!({"ids": [slope.0], "props": {"bodyType": "static"}}));
    let b = shape(&mut s, "shape.rectangle", 150.0, 100.0, 50.0, 30.0);
    ex(&mut s, "physics.set", json!({"ids": [b.0], "props": {"friction": 1, "restitution": 0}}));
    ex(&mut s, "select.set", json!({"ids": []}));
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert_eq!(out["moved"], json!([b.0]), "{out}");
    let n = s.doc().unwrap().doc.node(b).unwrap().clone();
    let slope_deg = (80.0f64 / 400.0).atan().to_degrees();
    // Counter-clockwise degrees, y down: going down to the right is a negative angle.
    assert!((n.bbox_angle + slope_deg).abs() < 1.5 || (n.bbox_angle - slope_deg).abs() < 1.5, "angle {}", n.bbox_angle);
}

#[test]
fn over_the_body_limit_dynamic_bodies_come_first_and_the_rest_are_reported() {
    let mut s = session();
    let mut statics = vec![];
    for i in 0..1_000 {
        statics.push(shape(&mut s, "shape.rectangle", f64::from(i % 40) * 10.0, 200.0 + f64::from(i / 40) * 3.0, 2.0, 2.0).0);
    }
    ex(&mut s, "physics.set", json!({"ids": statics, "props": {"bodyType": "static"}}));
    let falling = shape(&mut s, "shape.rectangle", 100.0, 20.0, 30.0, 30.0);
    ex(&mut s, "select.set", json!({"ids": [falling.0]}));
    let out = ex(&mut s, "physics.simulate", json!({"steps": 5}));
    assert_eq!(out["bodies"], 1_000, "{}", out["notes"]);
    assert_eq!(out["moved"], json!([falling.0]));
    let skipped = out["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0]["reason"].as_str().unwrap().contains("limit"));
}

#[test]
fn soft_settings_through_the_commands() {
    let mut s = session();
    let r = shape(&mut s, "shape.rectangle", 10.0, 10.0, 120.0, 20.0);
    let out = ex(&mut s, "physics.set", json!({"ids": [r.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth", "bend": 0.3}}}));
    assert_eq!(out["settings"]["bodyType"], "soft");
    assert_eq!(out["settings"]["soft"], json!({"preset": "cloth", "bend": 0.3}));
    // Fields merge into the soft settings; null goes back to the preset.
    let out = ex(&mut s, "physics.set", json!({"ids": [r.0], "props": {"soft": {"bend": null, "stiffness": 2}}}));
    assert_eq!(out["settings"]["soft"], json!({"preset": "cloth", "stiffness": 1.0}));
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");
    // Undone with the document (it is the object's attribute).
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "physics.get", json!({"id": r.0}))["settings"]["soft"]["bend"], 0.3);
}
