//! PICABIA: the live physics session (PIC31.4): open, play by ticks, seek, poke, Simulate to Rest,
//! Apply as one undo step, Cancel leaving the art alone, and restarting when the document changes.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Rect;

use super::session::{playing, tick};
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

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

/// A jelly ellipse, a rigid box and a cloth strip over a static shelf.
fn scene(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let jelly = shape(s, "shape.ellipse", 40.0, 30.0, 80.0, 60.0);
    ex(s, "physics.set", json!({"ids": [jelly.0], "props": {"bodyType": "soft", "soft": {"preset": "jelly"}}}));
    let boxy = shape(s, "shape.rectangle", 160.0, 20.0, 40.0, 40.0);
    ex(s, "physics.set", json!({"ids": [boxy.0], "props": {"bodyType": "dynamic"}}));
    let shelf = shape(s, "shape.rectangle", 260.0, 150.0, 80.0, 8.0);
    ex(s, "physics.set", json!({"ids": [shelf.0], "props": {"bodyType": "static"}}));
    let cloth = shape(s, "shape.rectangle", 220.0, 100.0, 160.0, 4.0);
    ex(s, "physics.set", json!({"ids": [cloth.0], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}}));
    ex(s, "select.set", json!({"ids": []}));
    (jelly, boxy, shelf, cloth)
}

#[test]
fn open_play_seek_and_cancel_leave_the_art_alone() {
    let mut s = session();
    let (jelly, ..) = scene(&mut s);
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let out = ex(&mut s, "physics.session.open", json!({}));
    assert_eq!((out["bodies"].as_u64(), out["soft"].as_u64()), (Some(4), Some(2)), "{out}");
    ex(&mut s, "physics.play", json!({}));
    assert!(playing(&s));
    // A second of frames at 60 fps.
    for _ in 0..60 {
        tick(&mut s, 1.0 / 60.0);
    }
    let info = ex(&mut s, "physics.session.info", json!({}));
    assert!(info["step"].as_u64().unwrap() >= 55, "{info}");
    ex(&mut s, "physics.pause", json!({}));
    ex(&mut s, "physics.seek", json!({"step": 200}));
    ex(&mut s, "physics.seek", json!({"step": 30}));
    assert_eq!(ex(&mut s, "physics.session.info", json!({}))["step"], 30);
    // Nothing in the document changed.
    assert!(s.doc().unwrap().doc == before && s.doc().unwrap().history.undo.len() == undo);
    ex(&mut s, "physics.cancel", json!({}));
    assert_eq!(ex(&mut s, "physics.session.info", json!({}))["open"], false);
    assert!(bounds(&s, jelly) == before.node(jelly).unwrap().geometric_bounds().unwrap());
}

#[test]
fn to_rest_then_apply_bakes_every_body_as_one_undo_step() {
    let mut s = session();
    let (jelly, boxy, shelf, cloth) = scene(&mut s);
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let out = ex(&mut s, "physics.toRest", json!({}));
    assert!(out["step"].as_u64().unwrap() > 30, "{out}");
    let out = ex(&mut s, "physics.apply", json!({}));
    let changed: Vec<u64> = out["changed"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert!(changed.contains(&jelly.0) && changed.contains(&boxy.0) && changed.contains(&cloth.0) && !changed.contains(&shelf.0), "{out}");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Apply Physics");
    assert_eq!(ex(&mut s, "physics.session.info", json!({}))["open"], false, "Apply closes the session");
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!((bounds(&s, jelly).y1 - board.y1).abs() < 8.0, "the jelly rests on the floor: {:?}", bounds(&s, jelly));
    assert!((bounds(&s, boxy).y1 - board.y1).abs() < 3.0, "the box too: {:?}", bounds(&s, boxy));
    let c = bounds(&s, cloth);
    assert!(c.height() > 30.0, "the cloth drapes over the shelf: {c:?}");
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc == before);
}

#[test]
fn pokes_replay_when_scrubbing_and_edits_restart_the_session() {
    let mut s = session();
    let (jelly, boxy, ..) = scene(&mut s);
    ex(&mut s, "physics.session.open", json!({}));
    ex(&mut s, "physics.seek", json!({"step": 30}));
    // Grab the box where it is now, and pull, at the same step (paused).
    let poses = s.picabia.physics.as_ref().unwrap().poses();
    let (_, pose, _) = poses.iter().find(|(id, ..)| *id == boxy).unwrap();
    let c = bounds(&s, boxy).center();
    let c = match pose {
        vectorcraft_physics::pose::Pose::Rigid(Some(xf)) => *xf * c,
        _ => c,
    };
    ex(&mut s, "physics.poke", json!({"id": boxy.0, "point": [c.x, c.y]}));
    ex(&mut s, "physics.poke", json!({"to": {"x": 300, "y": 40}}));
    ex(&mut s, "physics.seek", json!({"step": 90}));
    ex(&mut s, "physics.poke", json!({"release": true}));
    ex(&mut s, "physics.seek", json!({"step": 150}));
    let a = s.picabia.physics.as_ref().unwrap().sim.world().poses();
    // The poke changed the box's motion (an unpoked run ends elsewhere).
    let mut t = session();
    scene(&mut t);
    ex(&mut t, "physics.session.open", json!({}));
    ex(&mut t, "physics.seek", json!({"step": 150}));
    let unpoked = t.picabia.physics.as_ref().unwrap().sim.world().poses();
    let k = s.picabia.physics.as_ref().unwrap().body_of(boxy).unwrap();
    assert_ne!(a[k], unpoked[k], "the poke moved the box");
    ex(&mut s, "physics.seek", json!({"step": 0}));
    ex(&mut s, "physics.seek", json!({"step": 150}));
    let again = s.picabia.physics.as_ref().unwrap().sim.world().poses();
    assert_eq!(a, again, "the same motion, poke included");
    // Junk pokes are refused.
    assert!(s.execute("physics.poke", &json!({"id": 999_999, "point": [0, 0]})).is_err());
    assert!(s.execute("physics.poke", &json!({"id": jelly.0})).is_err());
    assert!(s.execute("physics.poke", &json!({"id": jelly.0, "point": [f64::MAX, 0]})).is_err());
    assert!(s.execute("physics.seek", &json!({"step": -1})).is_err());
    assert!(s.execute("physics.seek", &json!({})).is_err());
    // Moving an object (an edit) starts the session again from the new rest pose.
    ex(&mut s, "object.transform", json!({"ids": [jelly.0], "matrix": [1, 0, 0, 1, 10, 0]}));
    let info = ex(&mut s, "physics.session.info", json!({}));
    assert_eq!(info["step"], 0, "{info}");
    assert!(info["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("started again")), "{info}");
}

#[test]
fn simulate_drops_soft_bodies_too() {
    let mut s = session();
    let (jelly, ..) = scene(&mut s);
    let out = ex(&mut s, "physics.simulate", json!({}));
    assert!(out["moved"].as_array().unwrap().iter().any(|v| v.as_u64() == Some(jelly.0)), "{out}");
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!((bounds(&s, jelly).y1 - board.y1).abs() < 8.0, "{:?}", bounds(&s, jelly));
}

#[test]
fn the_session_belongs_to_its_document() {
    let mut s = session();
    scene(&mut s);
    ex(&mut s, "physics.session.open", json!({}));
    ex(&mut s, "file.new", json!({"width": 200, "height": 200}));
    assert_eq!(ex(&mut s, "physics.session.info", json!({}))["open"], false);
    assert!(s.execute("physics.apply", &json!({})).is_err());
    assert!(!tick(&mut s, 1.0));
    // Nothing to move in an empty document.
    assert!(s.execute("physics.session.open", &json!({})).is_err());
}

#[test]
fn the_display_document_shows_clones_over_dimmed_objects() {
    use super::session::{DIMMED, bar_state, body_at, display_doc};
    let mut s = session();
    let (jelly, boxy, shelf, _) = scene(&mut s);
    assert!(display_doc(&s).is_none() && bar_state(&s).is_none());
    ex(&mut s, "physics.session.open", json!({}));
    // At rest nothing moved: no clones yet.
    let count = |d: &vectorcraft_doc::Document| d.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum::<usize>();
    let doc = s.doc().unwrap().doc.clone();
    assert_eq!(count(&display_doc(&s).unwrap()), count(&doc));
    ex(&mut s, "physics.seek", json!({"step": 60}));
    let shown = display_doc(&s).unwrap();
    assert_eq!(count(&shown), count(&doc) + 3, "the jelly, the box and the cloth moved; the shelf didn't");
    assert!((shown.node(jelly).unwrap().opacity - doc.node(jelly).unwrap().opacity * DIMMED).abs() < 1e-9);
    assert_eq!(shown.node(shelf).unwrap().opacity, doc.node(shelf).unwrap().opacity);
    let bar = bar_state(&s).unwrap();
    assert_eq!((bar.step, bar.furthest, bar.playing, bar.bodies), (60, 60, false, 4));
    ex(&mut s, "physics.seek", json!({"step": 10}));
    assert_eq!(bar_state(&s).unwrap().furthest, 60, "the scrubber keeps its end");
    // Hit test on the shown pose: the box where it is now, not where it was.
    let poses = s.picabia.physics.as_ref().unwrap().poses();
    let (_, pose, _) = poses.iter().find(|(id, ..)| *id == boxy).unwrap();
    let vectorcraft_physics::pose::Pose::Rigid(Some(xf)) = pose else { panic!("the box moved") };
    let now = *xf * bounds(&s, boxy).center();
    assert_eq!(body_at(&s, now), Some(boxy));
    assert_eq!(body_at(&s, vectorcraft_geom::Point::new(-500.0, -500.0)), None);
    // The shelf (static) is never grabbed.
    assert_eq!(body_at(&s, bounds(&s, shelf).center()), None);
}
