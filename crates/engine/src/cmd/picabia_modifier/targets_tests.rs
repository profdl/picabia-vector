//! PICABIA: copy-to-shape targets through the commands (validation, warnings, the picker list),
//! target ids following duplicate, copying moves, copy and paste and `modifier.apply`, and
//! `modifier.duplicate`.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_modifiers::ModifierKind;

use super::tests::{data, err, ex, id_of, session, undo_labels};
use crate::Session;

fn shape(s: &mut Session, cmd: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&ex(s, cmd, json!({"x": x, "y": y, "width": w, "height": h}))["id"])
}

/// The target of `id`'s copy-to-shape row (the first one).
fn target_of(s: &Session, id: NodeId) -> String {
    let st = vectorcraft_modifiers::parse(&data(s, id).unwrap()).unwrap();
    st.modifiers
        .iter()
        .find_map(|m| match &m.kind {
            ModifierKind::CopyToShape(c) => Some(c.target.clone()),
            _ => None,
        })
        .unwrap()
}

fn t(id: NodeId) -> String {
    id.0.to_string()
}

/// A 20×20 source and an ellipse target beside it, the source copying 12 times along it.
fn along() -> (Session, NodeId, NodeId) {
    let mut s = session();
    let src = shape(&mut s, "shape.rectangle", 0.0, 0.0, 20.0, 20.0);
    let tgt = shape(&mut s, "shape.ellipse", 100.0, 0.0, 200.0, 100.0);
    let out =
        ex(&mut s, "modifier.add", json!({"ids": [src.0], "kind": "copyToShape", "props": {"target": t(tgt), "mode": "alongPath", "count": 12}}));
    assert_eq!(out["modifier"], "m1");
    (s, src, tgt)
}

#[test]
fn add_copy_to_shape_places_copies_on_the_target() {
    let (mut s, src, tgt) = along();
    let l = ex(&mut s, "modifier.list", json!({"id": src.0}));
    assert_eq!(l["instances"], 12, "{l}");
    assert_eq!(l["modifiers"][0]["target"], t(tgt));
    assert_eq!(l["warnings"], json!([]));
    // A numeric target reads too.
    let other = shape(&mut s, "shape.rectangle", 0.0, 300.0, 10.0, 10.0);
    ex(&mut s, "modifier.add", json!({"ids": [other.0], "kind": "copyToShape", "props": {"target": tgt.0, "mode": "points"}}));
    assert_eq!(target_of(&s, other), t(tgt));
}

#[test]
fn bad_targets_are_refused_and_roll_back() {
    let (mut s, src, tgt) = along();
    let inner = shape(&mut s, "shape.rectangle", 10.0, 510.0, 20.0, 20.0);
    let beside = shape(&mut s, "shape.rectangle", 100.0, 510.0, 20.0, 20.0);
    let group = id_of(&ex(&mut s, "object.group", json!({"ids": [inner.0, beside.0]}))["id"]);
    let layer = s.doc().unwrap().doc.layers[0].id;
    let cts = |t: Value| json!({"kind": "copyToShape", "props": {"target": t}});
    for (id, p, want) in [
        (src, cts(json!(t(src))), "onto itself"),
        (inner, cts(json!(t(group))), "contains this object"),
        (group, cts(json!(t(inner))), "inside this object"),
        (src, cts(json!("99999")), "gone"),
        (src, cts(json!(t(layer))), "top-level layer"),
        (src, cts(json!("x")), "isn't an object id"),
        // tgt → src makes a loop with src → tgt.
        (tgt, cts(json!(t(src))), "copies onto this object"),
    ] {
        let was = data(&s, id);
        let mut p = p;
        p["ids"] = json!([id.0]);
        let e = err(&mut s, "modifier.add", p.clone());
        assert!(e.contains(want), "{p}: {e}");
        assert_eq!(data(&s, id), was, "{p} changed nothing");
    }
    // Setting a bad target on an existing row is refused the same way.
    let e = err(&mut s, "modifier.set", json!({"ids": [src.0], "modifier": "m1", "props": {"target": t(src)}}));
    assert!(e.contains("onto itself"), "{e}");
    // Pasting the row onto its own target would loop: refused.
    ex(&mut s, "modifier.copy", json!({"id": src.0}));
    let e = err(&mut s, "modifier.paste", json!({"ids": [tgt.0]}));
    assert!(e.contains("onto itself") || e.contains("copies onto this object"), "{e}");
    // A transitive loop: c → src, then tgt → c (src → tgt → c → src).
    let c = shape(&mut s, "shape.rectangle", 0.0, 900.0, 10.0, 10.0);
    ex(&mut s, "modifier.add", json!({"ids": [c.0], "kind": "copyToShape", "props": {"target": t(src)}}));
    assert!(
        err(&mut s, "modifier.add", json!({"ids": [tgt.0], "kind": "copyToShape", "props": {"target": t(c)}})).contains("copies onto this object")
    );
    // Turning a row with a bad target back on is checked too.
    let d = shape(&mut s, "shape.rectangle", 0.0, 1200.0, 10.0, 10.0);
    ex(&mut s, "modifier.add", json!({"ids": [d.0], "kind": "copyToShape", "props": {"target": t(tgt), "enabled": false}}));
    ex(&mut s, "edit.clear", json!({"ids": [tgt.0]}));
    assert!(err(&mut s, "modifier.toggle", json!({"ids": [d.0], "index": 0})).contains("gone"));
}

#[test]
fn a_deleted_target_leaves_a_warning_and_other_edits_still_work() {
    let (mut s, src, tgt) = along();
    ex(&mut s, "edit.clear", json!({"ids": [tgt.0]}));
    let l = ex(&mut s, "modifier.list", json!({"id": src.0}));
    assert_eq!(l["instances"], 1, "{l}");
    assert!(l["warnings"][0]["message"].as_str().unwrap().contains("gone"), "{l}");
    // Unrelated edits on the object aren't blocked by the dangling target.
    ex(&mut s, "modifier.add", json!({"ids": [src.0], "kind": "mirror"}));
    ex(&mut s, "modifier.toggle", json!({"ids": [src.0], "index": 1}));
    let r = ex(&mut s, "modifier.targets", json!({"id": src.0, "index": 0}));
    assert_eq!((r["valid"].clone(), r["reason"].as_str().is_some_and(|r| r.contains("gone"))), (json!(false), true), "{r}");
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": src.0}))["instances"], 12, "undoing the delete brings the copies back");
}

#[test]
fn modifier_targets_lists_what_the_picker_offers() {
    let (mut s, src, tgt) = along();
    let other = shape(&mut s, "shape.rectangle", 0.0, 300.0, 10.0, 10.0);
    ex(&mut s, "modifier.add", json!({"ids": [other.0], "kind": "copyToShape", "props": {"target": t(src)}}));
    let r = ex(&mut s, "modifier.targets", json!({"id": src.0, "modifier": "m1"}));
    let ids: Vec<u64> = r["targets"].as_array().unwrap().iter().map(|r| r["id"].as_u64().unwrap()).collect();
    assert!(ids.contains(&tgt.0), "{r}");
    assert!(!ids.contains(&src.0), "not itself: {r}");
    assert!(!ids.contains(&other.0), "not an object copying onto it: {r}");
    assert_eq!((r["target"].clone(), r["valid"].clone()), (json!(t(tgt)), json!(true)));
    assert_eq!(r["targets"][0]["kind"], "Ellipse", "{r}");
    // The Rust API the panel calls agrees.
    let doc = &s.doc().unwrap().doc;
    let choices = super::target_choices(doc, src);
    assert!(choices.iter().any(|(id, name)| *id == tgt && !name.is_empty()));
    assert!(super::check_target(doc, src, other).is_err());
}

#[test]
fn duplicating_source_and_target_together_points_the_copy_at_the_copied_target() {
    let (mut s, src, tgt) = along();
    let out = ex(&mut s, "edit.duplicate", json!({"ids": [src.0, tgt.0]}));
    let copies: Vec<NodeId> = out["ids"].as_array().unwrap().iter().map(id_of).collect();
    let (new_src, new_tgt) = (copies[0], copies[1]);
    assert!(data(&s, new_src).is_some());
    assert_eq!(target_of(&s, new_src), t(new_tgt));
    assert_eq!(target_of(&s, src), t(tgt), "the original is unchanged");
    // The source alone keeps copying onto the original target.
    let alone = id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [src.0]}))["ids"][0]);
    assert_eq!(target_of(&s, alone), t(tgt));
    // A copying move of both, grouped, too.
    let g = id_of(&ex(&mut s, "object.group", json!({"ids": [src.0, tgt.0]}))["id"]);
    let gc = id_of(&ex(&mut s, "object.move", json!({"ids": [g.0], "dx": 0, "dy": 400, "copy": true}))["ids"][0]);
    let kids: Vec<NodeId> = s.doc().unwrap().doc.node(gc).unwrap().children().unwrap().iter().map(|c| c.id).collect();
    let (k_src, k_tgt) = if data(&s, kids[0]).is_some() { (kids[0], kids[1]) } else { (kids[1], kids[0]) };
    assert_eq!(target_of(&s, k_src), t(k_tgt));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": k_src.0}))["instances"], 12);
}

#[test]
fn copy_paste_of_source_and_target_points_the_paste_at_the_pasted_target() {
    let (mut s, src, tgt) = along();
    ex(&mut s, "select.set", json!({"ids": [src.0, tgt.0]}));
    ex(&mut s, "edit.copy", json!({}));
    let pasted: Vec<NodeId> = ex(&mut s, "edit.paste", json!({}))["ids"].as_array().unwrap().iter().map(id_of).collect();
    assert_eq!(pasted.len(), 2);
    let (p_src, p_tgt) = if data(&s, pasted[0]).is_some() { (pasted[0], pasted[1]) } else { (pasted[1], pasted[0]) };
    assert_eq!(target_of(&s, p_src), t(p_tgt));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": p_src.0}))["instances"], 12);
    // Only the source: it keeps the original target.
    ex(&mut s, "select.set", json!({"ids": [src.0]}));
    ex(&mut s, "edit.copy", json!({}));
    let only = id_of(&ex(&mut s, "edit.paste", json!({}))["ids"][0]);
    assert_eq!(target_of(&s, only), t(tgt));
    // Into another document, a target left behind names nothing there: it is cleared.
    ex(&mut s, "file.new", json!({"width": 500, "height": 500}));
    let there = id_of(&ex(&mut s, "edit.paste", json!({}))["ids"][0]);
    assert_eq!(target_of(&s, there), "");
}

#[test]
fn apply_points_targets_inside_each_copy_at_that_copy() {
    let mut s = session();
    let src = shape(&mut s, "shape.rectangle", 0.0, 0.0, 10.0, 10.0);
    let tgt = shape(&mut s, "shape.ellipse", 50.0, 0.0, 100.0, 50.0);
    ex(&mut s, "modifier.add", json!({"ids": [src.0], "kind": "copyToShape", "props": {"target": t(tgt), "count": 4}}));
    let g = id_of(&ex(&mut s, "object.group", json!({"ids": [src.0, tgt.0]}))["id"]);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "linearArray", "props": {"count": 2, "offsetY": 200}}));
    let baked = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [g.0]}))["ids"][0]);
    let doc = &s.doc().unwrap().doc;
    for copy in doc.node(baked).unwrap().children().unwrap() {
        let kids: Vec<NodeId> = copy.children().unwrap().iter().map(|c| c.id).collect();
        let (k_src, k_tgt) = if data(&s, kids[0]).is_some() { (kids[0], kids[1]) } else { (kids[1], kids[0]) };
        assert_eq!(target_of(&s, k_src), t(k_tgt));
    }
}

#[test]
fn duplicate_modifier_puts_a_copy_below_with_a_fresh_id() {
    let (mut s, src, _) = along();
    ex(&mut s, "modifier.add", json!({"ids": [src.0], "kind": "mirror"}));
    let d = ex(&mut s, "modifier.duplicate", json!({"ids": [src.0], "index": 0}));
    assert_eq!(d["modifier"], "m3");
    let l = ex(&mut s, "modifier.list", json!({"id": src.0}));
    let ids: Vec<&str> = l["modifiers"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["m1", "m3", "m2"]);
    assert_eq!(l["modifiers"][1]["count"], 12);
    assert_eq!(undo_labels(&s).last().unwrap(), "Duplicate Modifier");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": src.0}))["modifiers"].as_array().unwrap().len(), 2);
}
