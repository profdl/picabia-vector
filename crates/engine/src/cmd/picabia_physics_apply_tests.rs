//! PICABIA: Apply bakes rigid and soft poses into the art as one undo step (PIC31.3).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};
use vectorcraft_physics::pose::Pose;
use vectorcraft_physics::soft::{Preset, Soft};

use super::apply::{Baking, bake};
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn shape(s: &mut Session, cmd: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute(cmd, &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

#[test]
fn apply_moves_rigid_and_bends_soft_art_as_one_undo_step() {
    let mut s = session();
    let rigid = shape(&mut s, "shape.rectangle", 10.0, 10.0, 40.0, 40.0);
    let jelly = shape(&mut s, "shape.ellipse", 100.0, 10.0, 80.0, 50.0);
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let n = s.doc().unwrap().doc.node(jelly).unwrap().clone();
    let mesh = vectorcraft_physics::mesh::soft_mesh(&vectorcraft_physics::body_shape(&n, None).unwrap(), &Soft::of(Preset::Jelly), 5_000).unwrap();
    let dropped: Vec<Point> = mesh.rest.iter().map(|p| *p + Vec2::new(0.0, 150.0)).collect();
    let rigid_pose = Pose::Rigid(Some(Affine::translate((0.0, 200.0))));
    let soft_pose = Pose::Soft(dropped);
    let items = [Baking { id: rigid, pose: &rigid_pose, mesh: None }, Baking { id: jelly, pose: &soft_pose, mesh: Some(&mesh) }];
    let changed = bake(&mut s, "Apply Physics", &Value::Null, &items).unwrap();
    assert_eq!(changed, vec![rigid, jelly]);
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Apply Physics");
    let r = bounds(&s, rigid);
    assert!((r.y0 - 210.0).abs() < 1e-6, "{r:?}");
    let j = bounds(&s, jelly);
    assert!((j.y0 - 160.0).abs() < 0.5 && (j.x0 - 100.0).abs() < 0.5, "{j:?}");
    // The rigid rectangle is still a live shape; the bent ellipse is a plain path.
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(&doc.node(rigid).unwrap().kind, NodeKind::Path { live: Some(_), .. }));
    assert!(matches!(&doc.node(jelly).unwrap().kind, NodeKind::Path { live: None, .. }));
    // Fills and strokes kept.
    assert_eq!(doc.node(jelly).unwrap().appearance, before.node(jelly).unwrap().appearance);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc == before);
}

#[test]
fn nothing_moved_makes_no_undo_step() {
    let mut s = session();
    let r = shape(&mut s, "shape.rectangle", 10.0, 10.0, 40.0, 40.0);
    let undo = s.doc().unwrap().history.undo.len();
    let still = Pose::Rigid(None);
    let n = s.doc().unwrap().doc.node(r).unwrap().clone();
    let mesh = vectorcraft_physics::mesh::soft_mesh(&vectorcraft_physics::body_shape(&n, None).unwrap(), &Soft::of(Preset::Cloth), 5_000).unwrap();
    let wrong = Pose::Soft(vec![Point::ZERO; 3]);
    let items = [
        Baking { id: r, pose: &still, mesh: None },
        Baking { id: r, pose: &wrong, mesh: Some(&mesh) },
        Baking { id: NodeId(9_999), pose: &still, mesh: None },
    ];
    assert!(bake(&mut s, "Apply Physics", &Value::Null, &items).unwrap().is_empty());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn duplicates_layers_and_nested_objects_are_baked_once() {
    let mut s = session();
    let a = shape(&mut s, "shape.rectangle", 10.0, 10.0, 40.0, 40.0);
    let b = shape(&mut s, "shape.rectangle", 60.0, 10.0, 40.0, 40.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let layer = s.doc().unwrap().doc.layers[0].id;
    let down = Pose::Rigid(Some(Affine::translate((0.0, 100.0))));
    let items = [
        Baking { id: group, pose: &down, mesh: None },
        Baking { id: group, pose: &down, mesh: None },
        Baking { id: a, pose: &down, mesh: None },
        Baking { id: layer, pose: &down, mesh: None },
    ];
    let changed = bake(&mut s, "Apply Physics", &Value::Null, &items).unwrap();
    assert_eq!(changed, vec![group]);
    assert!((bounds(&s, a).y0 - 110.0).abs() < 1e-6, "moved once, with its group: {:?}", bounds(&s, a));
}
