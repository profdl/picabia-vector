//! PICABIA: `modifier.apply` tests.

use std::collections::BTreeSet;

use serde_json::json;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::Affine;

use super::tests::{data, err, ex, id_of, put, rect, session, undo_labels};
use crate::{EngineError, Session};

fn add(s: &mut Session, id: NodeId, kind: &str, props: serde_json::Value) {
    ex(s, "modifier.add", json!({"ids": [id.0], "kind": kind, "props": props}));
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

fn node(s: &Session, id: NodeId) -> Node {
    doc(s).node(id).unwrap_or_else(|| panic!("no node {id:?}")).clone()
}

fn children(s: &Session, id: NodeId) -> Vec<NodeId> {
    node(s, id).children().unwrap().iter().map(|c| c.id).collect()
}

fn has_stack(s: &Session, id: NodeId) -> bool {
    vectorcraft_modifiers::has_modifiers(&node(s, id))
}

fn scaling() -> Scaling {
    Scaling { strokes: true, effects: Some(vectorcraft_render::effects::scale_effect), patterns: true, ..Scaling::default() }
}

/// `n` moved by `t` (the way a drawn copy is).
fn moved(n: &Node, t: Affine) -> Node {
    let mut c = n.clone();
    c.transform(t, scaling());
    c
}

/// The bounds `n` draws at: one per modifier copy, or its own (rounded, sorted: compared as sets).
fn drawn_nodes(n: &Node) -> Vec<Node> {
    match vectorcraft_modifiers::evaluate_node(n) {
        Some(o) => o.instances.iter().map(|i| moved(n, i.transform)).collect(),
        None => vec![n.clone()],
    }
}

fn key(nodes: &[Node]) -> Vec<[i64; 4]> {
    let mut k: Vec<[i64; 4]> = nodes
        .iter()
        .map(|n| {
            let b = n.geometric_bounds().unwrap();
            [b.x0, b.y0, b.x1, b.y1].map(|c| (c * 1000.0).round() as i64)
        })
        .collect();
    k.sort();
    k
}

/// What object `id` draws, with its modifier copies.
fn drawn(s: &Session, id: NodeId) -> Vec<[i64; 4]> {
    key(&drawn_nodes(&node(s, id)))
}

/// What a baked group draws: each child at each of the group's own live copies.
fn drawn_group(s: &Session, g: NodeId) -> Vec<[i64; 4]> {
    let gn = node(s, g);
    let copies: Vec<Affine> = vectorcraft_modifiers::evaluate_node(&gn)
        .map(|o| o.instances.iter().map(|i| i.transform).collect())
        .unwrap_or_else(|| vec![Affine::IDENTITY]);
    let mut out = Vec::new();
    for t in copies {
        for c in gn.children().unwrap() {
            out.push(moved(c, t));
        }
    }
    key(&out)
}

fn named(s: &mut Session, id: NodeId, name: &str) {
    ex(s, "object.setProps", json!({"ids": [id.0], "name": name}));
}

#[test]
fn apply_makes_a_group_of_named_copies_with_baked_transforms() {
    let mut s = session();
    let below = rect(&mut s, -300.0);
    let r = rect(&mut s, 0.0);
    let above = rect(&mut s, 500.0);
    let layer = doc(&s).parent_of(r).unwrap();
    named(&mut s, r, "Tile");
    add(&mut s, r, "linearArray", json!({"count": 5, "offsetX": 120}));
    let want = drawn(&s, r);
    let before = doc(&s).clone();
    ex(&mut s, "select.set", json!({"ids": [r.0]}));
    let out = ex(&mut s, "modifier.apply", json!({}));
    let g = id_of(&out["ids"][0]);
    assert_eq!(out["applied"][0]["copies"], 5);
    assert_eq!(undo_labels(&s).last().unwrap(), "Apply Modifiers");
    assert_eq!(s.doc().unwrap().selection.objects, [g], "the group is selected");
    assert!(doc(&s).node(r).is_none(), "the source is replaced");
    assert_eq!(children(&s, layer), [below, g, above], "same parent, same place in the stacking order");
    let gn = node(&s, g);
    assert!(matches!(gn.kind, NodeKind::Group { .. }));
    assert_eq!(gn.name.as_deref(), Some("Tile"));
    assert!(!has_stack(&s, g), "nothing left live");
    let names: Vec<String> = children(&s, g).iter().map(|c| node(&s, *c).name.unwrap()).collect();
    assert_eq!(names, ["Tile 1", "Tile 2", "Tile 3", "Tile 4", "Tile 5"]);
    for c in children(&s, g) {
        assert!(!has_stack(&s, c), "copies carry no stack");
    }
    assert_eq!(drawn_group(&s, g), want, "the copies sit where they were drawn");
    let xs: Vec<f64> = children(&s, g).iter().map(|c| node(&s, *c).geometric_bounds().unwrap().x0).collect();
    assert_eq!(xs, [0.0, 120.0, 240.0, 360.0, 480.0]);
    // Undo restores the document exactly.
    ex(&mut s, "edit.undo", json!({}));
    assert!(*doc(&s) == before, "undo restores the document");
    assert_eq!(s.doc().unwrap().selection.objects, [r]);
    ex(&mut s, "edit.redo", json!({}));
    assert!(doc(&s).node(g).is_some() && doc(&s).node(r).is_none());
    // The baked copies keep no Picabia attribute for SVG to write.
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg"}))["text"].as_str().unwrap().to_string();
    assert!(!svg.contains(vectorcraft_modifiers::PLUGIN_KEY), "{svg}");
}

#[test]
fn rotated_and_mirrored_copies_bake_exactly() {
    let mut s = session();
    let r = rect(&mut s, 40.0);
    ex(&mut s, "object.rotate", json!({"ids": [r.0], "angle": 30}));
    add(&mut s, r, "circularArray", json!({"count": 6, "radius": 150}));
    add(&mut s, r, "mirror", json!({"axis": "horizontal", "offset": 80}));
    add(&mut s, r, "randomize", json!({"seed": 7, "rotation": 40, "scale": 0.3}));
    let want = drawn(&s, r);
    assert_eq!(want.len(), 12);
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    assert_eq!(children(&s, g).len(), 12);
    assert_eq!(drawn_group(&s, g), want);
}

#[test]
fn strokes_scale_with_the_copy() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    ex(&mut s, "stroke.set", json!({"ids": [r.0], "weight": 4}));
    add(&mut s, r, "linearArray", json!({"count": 2, "scaleEach": 2}));
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    let weights: Vec<f64> = children(&s, g).iter().map(|c| node(&s, *c).appearance.stroke().unwrap().width).collect();
    assert_eq!(weights, [4.0, 8.0]);
}

#[test]
fn up_to_bakes_a_prefix_and_the_rest_stays_live_on_the_group() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, "linearArray", json!({"count": 3, "offsetX": 150}));
    // Without Align: an aligned ring turns each block about the group's centre (see `apply.rs`).
    add(&mut s, r, "circularArray", json!({"count": 4, "radius": 300, "align": false}));
    add(&mut s, r, "mirror", json!({"axis": "vertical", "offset": 900}));
    let want = drawn(&s, r);
    assert_eq!(want.len(), 24);
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0], "upTo": 0}));
    let g = id_of(&out["ids"][0]);
    assert_eq!(children(&s, g).len(), 3, "only the linear array is baked");
    let l = ex(&mut s, "modifier.list", json!({"id": g.0}));
    let kinds: Vec<&str> = l["modifiers"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["circularArray", "mirror"], "the rows after upTo move onto the group");
    assert_eq!(l["instances"], 8);
    assert_eq!(drawn_group(&s, g), want, "the result looks unchanged");
    // upTo past the end is refused; upTo at the end bakes everything.
    let r2 = rect(&mut s, 1500.0);
    add(&mut s, r2, "gridArray", json!({"rows": 2, "cols": 2}));
    assert!(err(&mut s, "modifier.apply", json!({"ids": [r2.0], "upTo": 1})).contains("upTo"));
    let g2 = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r2.0], "upTo": 0}))["ids"][0]);
    assert_eq!(children(&s, g2).len(), 4);
    assert!(!has_stack(&s, g2));
}

#[test]
fn nested_objects_get_new_unique_ids_and_keep_their_own_stacks() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let b = rect(&mut s, 150.0);
    add(&mut s, b, "mirror", json!({}));
    let f = id_of(&ex(&mut s, "object.group", json!({"ids": [a.0, b.0]}))["id"]);
    add(&mut s, f, "linearArray", json!({"count": 3, "offsetY": 300}));
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [f.0]}))["ids"][0]);
    let mut all = vec![];
    node(&s, g).walk(&mut |n| all.push(n.id));
    assert_eq!(all.len(), 1 + 3 * 3);
    let set: BTreeSet<NodeId> = all.iter().copied().collect();
    assert_eq!(set.len(), all.len(), "ids are unique");
    for old in [f, a, b] {
        assert!(!set.contains(&old) && doc(&s).node(old).is_none());
    }
    for copy in children(&s, g) {
        assert!(!has_stack(&s, copy), "the applied stack is gone from the copy");
        let inner = children(&s, copy);
        assert!(has_stack(&s, inner[1]), "a member's own stack is not this apply's");
        assert_eq!(doc(&s).parent_of(inner[1]), Some(copy));
    }
}

#[test]
fn nothing_to_apply_is_refused_cleanly() {
    let mut s = session();
    let plain = rect(&mut s, 0.0);
    let off = rect(&mut s, 200.0);
    add(&mut s, off, "linearArray", json!({"count": 4}));
    ex(&mut s, "modifier.toggle", json!({"ids": [off.0], "index": 0}));
    let one = rect(&mut s, 400.0);
    add(&mut s, one, "linearArray", json!({"count": 1}));
    let labels = undo_labels(&s).len();
    for id in [plain, off, one] {
        match s.execute("modifier.apply", &json!({"ids": [id.0]})) {
            Err(EngineError::Disabled(..)) => {}
            other => panic!("{id:?}: {other:?}"),
        }
        assert!(doc(&s).node(id).is_some());
    }
    assert_eq!(undo_labels(&s).len(), labels, "no undo step");
    // Mixed: the objects with copies are applied, the others left alone.
    let live = rect(&mut s, 600.0);
    add(&mut s, live, "linearArray", json!({"count": 2}));
    let out = ex(&mut s, "modifier.apply", json!({"ids": [plain.0, live.0]}));
    assert_eq!(out["ids"].as_array().unwrap().len(), 1);
    assert!(doc(&s).node(plain).is_some() && doc(&s).node(live).is_none());
}

#[test]
fn apply_refuses_more_objects_than_the_cap_before_copying() {
    let mut s = session();
    let ids: Vec<u64> = (0..10).map(|i| rect(&mut s, f64::from(i)).0).collect();
    let f = id_of(&ex(&mut s, "object.group", json!({"ids": ids}))["id"]);
    add(&mut s, f, "gridArray", json!({"rows": 100, "cols": 100}));
    let nodes = doc(&s).node_count();
    let e = err(&mut s, "modifier.apply", json!({"ids": [f.0]}));
    assert!(e.contains(&super::apply::MAX_APPLY_OBJECTS.to_string()), "{e}");
    assert_eq!(doc(&s).node_count(), nodes);
}

#[test]
fn a_damaged_stack_and_compound_members_are_refused() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    put(&mut s, r, "{broken");
    assert!(s.execute("modifier.apply", &json!({"ids": [r.0]})).is_err());
    assert!(doc(&s).node(r).is_some());
    let (a, b) = (rect(&mut s, 300.0), rect(&mut s, 350.0));
    ex(&mut s, "object.compoundPath.make", json!({"ids": [a.0, b.0]}));
    let c = doc(&s).parent_of(a).unwrap();
    assert!(matches!(node(&s, c).kind, NodeKind::Compound { .. }));
    let member = children(&s, c)[0];
    add(&mut s, member, "linearArray", json!({"count": 2}));
    assert!(err(&mut s, "modifier.apply", json!({"ids": [member.0]})).contains("part of another object"));
    assert!(data(&s, member).is_some());
}

#[test]
fn a_stencil_row_stays_live_and_only_the_rows_above_it_are_baked() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, "linearArray", json!({"count": 3, "offsetX": 150}));
    add(&mut s, r, "stencil", json!({"widthMm": 200}));
    add(&mut s, r, "mirror", json!({}));
    let want = drawn(&s, r);
    assert_eq!(want.len(), 3, "the mirror below the stencil is ignored");
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    assert_eq!(children(&s, g).len(), 3, "the copies above the stencil are baked");
    let l = ex(&mut s, "modifier.list", json!({"id": g.0}));
    let kinds: Vec<&str> = l["modifiers"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["stencil", "mirror"], "the stencil and the rows below it move onto the group");
    assert_eq!(l["modifiers"][0]["widthMm"], json!(200.0));
    assert_eq!(drawn_group(&s, g), want, "the result looks unchanged");
    // A stencil on top: nothing to bake.
    let r2 = rect(&mut s, 1500.0);
    add(&mut s, r2, "stencil", json!({}));
    add(&mut s, r2, "gridArray", json!({"rows": 2, "cols": 2}));
    assert!(matches!(s.execute("modifier.apply", &json!({"ids": [r2.0]})), Err(EngineError::Disabled(..))));
    assert_eq!(drawn(&s, r2).len(), 1, "a stencil alone draws the object once, unchanged");
}
