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
    // Each copy's outline, point by point: the live instance transform applied to the source's
    // outline (computed here with the geometry crate, not with `Node::transform`).
    let src = node(&s, r);
    let outline = src.path_data().unwrap().to_bezpath();
    let instances: Vec<Affine> = vectorcraft_modifiers::evaluate_node_in_doc(doc(&s), &src).unwrap().instances.iter().map(|i| i.transform).collect();
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    assert_eq!(children(&s, g).len(), 12);
    assert_eq!(drawn_group(&s, g), want);
    for (c, t) in children(&s, g).iter().zip(&instances) {
        let got = node(&s, *c).path_data().unwrap().to_bezpath();
        let mut expected = outline.clone();
        expected.apply_affine(*t);
        let pts = |b: &vectorcraft_geom::BezPath| -> Vec<vectorcraft_geom::Point> { b.elements().iter().filter_map(|e| e.end_point()).collect() };
        let (a, b) = (pts(&got), pts(&expected));
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(&b) {
            assert!(p.distance(*q) < 1e-6, "{p:?} vs {q:?}");
        }
    }
}

#[test]
fn a_copys_effects_and_strokes_scale_with_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    ex(&mut s, "stroke.set", json!({"ids": [r.0], "weight": 3}));
    s.edit("Shadow", |d, _| {
        d.node_mut(r).unwrap().appearance.effects.push(vectorcraft_doc::Effect {
            id: "stylize.dropShadow".into(),
            params: json!({"x": 8.0, "y": 6.0, "blur": 4.0, "opacity": 75.0}),
            visible: true,
        });
        Ok(())
    })
    .unwrap();
    add(&mut s, r, "linearArray", json!({"count": 3, "scaleEach": 2}));
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    let got: Vec<(f64, f64, f64)> = children(&s, g)
        .iter()
        .map(|c| {
            let n = node(&s, *c);
            let fx = &n.appearance.effects[0].params;
            (n.appearance.stroke().unwrap().width, fx["x"].as_f64().unwrap(), fx["blur"].as_f64().unwrap())
        })
        .collect();
    assert_eq!(got, [(3.0, 8.0, 4.0), (6.0, 16.0, 8.0), (12.0, 32.0, 16.0)]);
}

#[test]
fn the_first_member_of_a_clip_group_is_refused_whatever_its_kind() {
    let mut s = session();
    // A compound path clipping a rectangle.
    let a = rect(&mut s, 0.0);
    let (p, q) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    ex(&mut s, "object.compoundPath.make", json!({"ids": [p.0, q.0]}));
    let c = doc(&s).parent_of(p).unwrap();
    ex(&mut s, "object.clippingMask.make", json!({"ids": [a.0, c.0]}));
    let clip = doc(&s).parent_of(c).unwrap();
    assert_eq!(children(&s, clip)[0], c, "the compound path clips");
    add(&mut s, c, "linearArray", json!({"count": 3}));
    assert!(err(&mut s, "modifier.apply", json!({"ids": [c.0]})).contains("clipping path"));
    assert!(doc(&s).node(c).is_some());
    // Type clipping a rectangle.
    let b = rect(&mut s, 300.0);
    let t = id_of(&ex(&mut s, "text.create", json!({"x": 300, "y": 50, "text": "Clip", "size": 48}))["id"]);
    ex(&mut s, "object.clippingMask.make", json!({"ids": [b.0, t.0]}));
    let clip = doc(&s).parent_of(t).unwrap();
    assert_eq!(children(&s, clip)[0], t);
    add(&mut s, t, "linearArray", json!({"count": 2}));
    assert!(err(&mut s, "modifier.apply", json!({"ids": [t.0]})).contains("clipping path"));
    // The clipped art itself can be applied.
    add(&mut s, b, "linearArray", json!({"count": 2}));
    ex(&mut s, "modifier.apply", json!({"ids": [b.0]}));
    assert_eq!(children(&s, clip)[0], t, "the clipping object keeps its place");
}

#[test]
fn an_object_given_twice_is_applied_once() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, "linearArray", json!({"count": 3, "offsetX": 120}));
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0, r.0]}));
    assert_eq!(out["applied"].as_array().unwrap().len(), 1);
    assert_eq!(children(&s, id_of(&out["ids"][0])).len(), 3);
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
fn applying_up_to_a_row_above_a_stencil_keeps_the_stencil_live() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, "linearArray", json!({"count": 3, "offsetX": 150}));
    add(&mut s, r, "stencil", json!({"widthMm": 200}));
    add(&mut s, r, "mirror", json!({}));
    let want = drawn(&s, r);
    assert_eq!(want.len(), 3, "the mirror below the stencil is ignored");
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0], "upTo": 0}))["ids"][0]);
    assert_eq!(children(&s, g).len(), 3, "the copies above the stencil are baked");
    let l = ex(&mut s, "modifier.list", json!({"id": g.0}));
    let kinds: Vec<&str> = l["modifiers"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["stencil", "mirror"], "the stencil and the rows below it move onto the group");
    assert_eq!(l["modifiers"][0]["widthMm"], json!(200.0));
    assert_eq!(drawn_group(&s, g), want, "the result looks unchanged");
    // Applying the whole stack bakes the stencil too: its sheet replaces the group.
    let p = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [g.0]}))["ids"][0]);
    assert!(matches!(node(&s, p).kind, NodeKind::Path { .. }) && !has_stack(&s, p), "the sheet, as a path");
}

/// Applying changes nothing on the page: the baked group draws what the live copies drew,
/// transparency (one composite for all the copies), effects and overlaps included.
#[test]
fn apply_draws_exactly_what_the_live_copies_drew() {
    let mut s = session();
    let r = rect(&mut s, 50.0);
    ex(&mut s, "object.setProps", json!({"ids": [r.0], "opacity": 50}));
    s.edit("Shadow", |d, _| {
        d.node_mut(r).unwrap().appearance.effects.push(vectorcraft_doc::Effect {
            id: "stylize.dropShadow".into(),
            params: json!({"x": 6.0, "y": 6.0, "blur": 3.0, "opacity": 75.0}),
            visible: true,
        });
        Ok(())
    })
    .unwrap();
    add(&mut s, r, "linearArray", json!({"count": 4, "offsetX": 60, "rotateEach": 10, "scaleEach": 1.1}));
    let region = vectorcraft_geom::Rect::new(0.0, 0.0, 500.0, 220.0);
    let draw = |s: &Session| vectorcraft_render::Renderer::new().render_region(doc(s), region, 1.0, true).pixels;
    let live = draw(&s);
    let g = id_of(&ex(&mut s, "modifier.apply", json!({"ids": [r.0]}))["ids"][0]);
    assert!((node(&s, g).opacity - 0.5).abs() < 1e-6, "the group takes the object's opacity");
    assert!(children(&s, g).iter().all(|c| node(&s, *c).opacity == 1.0), "the copies are opaque inside it");
    assert!(draw(&s) == live, "the baked group draws as the live copies did");
}

/// The first fill's paint of `n`.
fn fill_of(n: &Node) -> vectorcraft_color::Paint {
    n.appearance
        .items
        .iter()
        .find_map(|i| match i {
            vectorcraft_doc::AppearanceItem::Fill(f) => Some(f.paint.clone()),
            _ => None,
        })
        .unwrap()
}

/// PIC32.2: colour rows bake the shifted colours the renderer draws; a colour row alone applies.
#[test]
fn apply_bakes_the_copies_colours() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, "linearArray", json!({"count": 3, "offsetX": 150}));
    add(&mut s, r, "color", json!({"hue": 0, "chroma": 0, "lightness": -40}));
    let drawn = vectorcraft_render::picabia::copies_art(doc(&s), &node(&s, r)).unwrap();
    let want: Vec<_> = drawn.children().unwrap().iter().map(|c| fill_of(c)).collect();
    assert_ne!(want[0], want[2], "the copies differ");
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    let g = id_of(&out["ids"][0]);
    let got: Vec<_> = children(&s, g).iter().map(|c| fill_of(&node(&s, *c))).collect();
    assert_eq!(got, want, "baked as drawn");
    // A colour row alone makes a change: one recoloured copy.
    let r2 = rect(&mut s, 600.0);
    add(&mut s, r2, "color", json!({"hue": 0, "chroma": 0, "lightness": -40}));
    let before = fill_of(&node(&s, r2));
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r2.0]}));
    let g2 = id_of(&out["ids"][0]);
    let kids = children(&s, g2);
    assert_eq!(kids.len(), 1);
    assert_ne!(fill_of(&node(&s, kids[0])), before);
}
