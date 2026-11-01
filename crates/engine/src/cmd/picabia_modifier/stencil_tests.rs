//! PICABIA: stencil sheets in the engine (compute, freshness, background refresh), and
//! `modifier.stencilInfo` / `modifier.stencilFromSelection`.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_geom::kurbo::Shape as _;

use super::tests::{ex, id_of, session, undo_labels};
use crate::Session;
use crate::picabia_stencil as ps;

pub(super) const BLACK: &str = "#000000";
pub(super) const WHITE: &str = "#ffffff";

/// A `w` × `h` rectangle at (`x`, `y`) filled with `hex`, no stroke.
pub(super) fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, hex: &str) -> NodeId {
    let id = id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"]);
    ex(s, "paint.setFill", json!({"ids": [id.0], "color": hex}));
    ex(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

/// A black and a white 100-unit square made a stencil (300 mm wide by default).
pub(super) fn two_tone(s: &mut Session, props: Value) -> NodeId {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0, BLACK);
    let b = rect(s, 100.0, 0.0, 100.0, 100.0, WHITE);
    id_of(&ex(s, "modifier.stencilFromSelection", json!({"ids": [a.0, b.0], "props": props}))["id"])
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

#[test]
fn stencil_info_reports_colours_stats_and_size() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, BLACK);
    let b = rect(&mut s, 100.0, 0.0, 100.0, 100.0, WHITE);
    let none = ex(&mut s, "modifier.stencilInfo", json!({"id": a.0}));
    assert_eq!(none["stencil"], Value::Null);
    let made = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [a.0, b.0]}));
    let g = id_of(&made["id"]);
    assert_eq!(made["modifier"], "m1");
    let i = ex(&mut s, "modifier.stencilInfo", json!({"id": g.0}));
    assert_eq!(i["stencil"], "m1");
    assert_eq!(i["id"], g.0);
    let colours = i["colours"].as_array().unwrap();
    assert_eq!(colours.len(), 2, "{i}");
    let white = colours.iter().find(|c| c["hex"] == "#ffffff").unwrap();
    assert_eq!(white["cut"], true);
    assert!((white["area"].as_f64().unwrap() - 150.0 * 150.0).abs() < 1.0, "100 units at 1.5 mm/unit: {white}");
    assert_eq!(i["stats"]["cutPieces"], 5, "the white square and four registration holes: {i}");
    assert!(i["stats"]["cutLengthMm"].as_f64().unwrap() > 0.0);
    // 200 × 100 design, 300 mm wide, plus 15 mm all round.
    assert!((i["sheetMm"]["w"].as_f64().unwrap() - 330.0).abs() < 1e-6, "{i}");
    assert!((i["sheetMm"]["h"].as_f64().unwrap() - 180.0).abs() < 1e-6, "{i}");
    assert_eq!(i["shapes"], 2);
    assert_eq!(i["hoshi"].as_array().unwrap().len(), 4);
    assert_eq!(i["materials"].as_array().unwrap().len(), 3, "paper, film and card (custom has no preset)");
    assert_eq!(i["tile"], Value::Null);
    assert_eq!(i["repeat"], Value::Null);
    // The sheet the renderer will draw is the same one, fresh.
    let doc = s.doc().unwrap().doc.clone();
    let n = doc.node(g).unwrap();
    let (sheet, fresh) = ps::lookup(&doc, n).unwrap();
    assert!(fresh);
    assert!(Arc::ptr_eq(&sheet, &ps::stencil_sheet(&doc, n).unwrap()));
    assert!(sheet.cut.area().abs() > 0.0 && sheet.paper.area().abs() > 0.0);
    // Not a stencil: no sheet for the renderer.
    assert!(ps::stencil_sheet(&doc, doc.node(a).unwrap()).is_none());
}

#[test]
fn stencil_from_selection_groups_then_adds_the_row_in_one_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, BLACK);
    let b = rect(&mut s, 150.0, 0.0, 100.0, 100.0, WHITE);
    let steps = undo_labels(&s).len();
    let made = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [b.0, a.0], "props": {"widthMm": 500}}));
    let g = id_of(&made["id"]);
    assert_eq!(undo_labels(&s).len(), steps + 1);
    assert_eq!(undo_labels(&s).last().unwrap(), "Make Stencil");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), vec![g]);
    let n = node(&s, g);
    assert!(matches!(n.kind, NodeKind::Group { .. }));
    assert_eq!(n.name.as_deref(), Some("Stencil"));
    let kids: Vec<NodeId> = n.children().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(kids, vec![a, b], "paint order kept, whatever order they were named in");
    let st = vectorcraft_modifiers::node_stack(&n).unwrap().unwrap();
    assert_eq!(st.stencil().unwrap().1.width_mm, 500.0);
    // Again: refused, it is one already.
    assert!(s.execute("modifier.stencilFromSelection", &json!({"ids": [g.0]})).is_err());
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.node(g).is_none(), "one undo removes the group and the row");
    assert!(s.doc().unwrap().doc.node(a).is_some());
    // One object takes the row itself.
    let made = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [a.0]}));
    assert_eq!(id_of(&made["id"]), a);
    assert!(ps::is_stencil(&node(&s, a)));
    // Nothing selected, a layer: refused.
    ex(&mut s, "select.none", json!({}));
    assert!(s.execute("modifier.stencilFromSelection", &json!({})).is_err());
    let layer = s.doc().unwrap().doc.layers[0].id;
    assert!(s.execute("modifier.stencilFromSelection", &json!({"ids": [layer.0]})).is_err());
}

#[test]
fn a_ring_around_an_island_reports_it_tied_through_stencil_info() {
    // A black square with a white ring cut out of it and a black island in the ring.
    let mut s = session();
    let ids: Vec<u64> =
        [(0.0, 100.0, BLACK), (20.0, 60.0, WHITE), (40.0, 20.0, BLACK)].into_iter().map(|(x, w, hex)| rect(&mut s, x, x, w, w, hex).0).collect();
    let g = id_of(&ex(&mut s, "modifier.stencilFromSelection", json!({"ids": ids}))["id"]);
    let i = ex(&mut s, "modifier.stencilInfo", json!({"id": g.0}));
    let islands = i["islands"].as_array().unwrap();
    assert_eq!(islands.len(), 1, "{i}");
    let isl = &islands[0];
    for (k, want) in [("x", 40.0), ("y", 40.0), ("w", 20.0), ("h", 20.0)] {
        assert!((isl[k].as_f64().unwrap() - want).abs() < 1.0, "{k}, document space: {isl}");
    }
    assert_eq!(isl["depth"], 0);
    assert_eq!(isl["tied"], 2, "two ties by default: {isl}");
    assert_eq!(i["stats"]["islands"], 1);
    assert_eq!(i["stats"]["ties"], 2);
    assert_eq!(i["stats"]["untied"], 0);
    // The tie bars are kept for the canvas markers.
    let doc = s.doc().unwrap().doc.clone();
    let (sheet, _) = ps::lookup(&doc, doc.node(g).unwrap()).unwrap();
    assert!(sheet.info.ties.area().abs() > 0.0);
}

#[test]
fn results_stay_fresh_until_the_design_changes_and_preview_colours_only_rekey() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let doc = s.doc().unwrap().doc.clone();
    let first = ps::ensure_one(&doc, g).unwrap();
    // Moving something else keeps it fresh: the object is the same pointer.
    let other = rect(&mut s, 500.0, 500.0, 10.0, 10.0, BLACK);
    ex(&mut s, "object.move", json!({"ids": [other.0], "dx": 5, "dy": 0}));
    let doc = s.doc().unwrap().doc.clone();
    assert!(ps::lookup(&doc, doc.node(g).unwrap()).unwrap().1, "an unrelated edit keeps the sheet fresh");
    // A preview colour: stale until the job sees the same geometry, then the kept sheet is reused.
    ex(&mut s, "modifier.set", json!({"ids": [g.0], "index": 0, "props": {"paperColour": "#123456"}}));
    let doc = s.doc().unwrap().doc.clone();
    let (stale, fresh) = ps::lookup(&doc, doc.node(g).unwrap()).unwrap();
    assert!(!fresh && Arc::ptr_eq(&stale, &first), "the latest result draws meanwhile");
    let generation = ps::generation();
    let again = ps::ensure_one(&doc, g).unwrap();
    assert!(Arc::ptr_eq(&again, &first), "same geometry: re-keyed, not recomputed");
    assert!(ps::generation() > generation);
    // Moving the design changes it: a new sheet.
    ex(&mut s, "object.move", json!({"ids": [g.0], "dx": 10, "dy": 0}));
    let doc = s.doc().unwrap().doc.clone();
    assert!(!ps::lookup(&doc, doc.node(g).unwrap()).unwrap().1);
    let moved = ps::ensure_one(&doc, g).unwrap();
    assert!(!Arc::ptr_eq(&moved, &first));
    assert!((moved.sheet.x0 - first.sheet.x0 - 10.0).abs() < 1e-6, "{:?} vs {:?}", moved.sheet, first.sheet);
    // Undo: back to the first design (found again by its geometry while the store still keeps
    // it; tests running alongside share the store, so only the sheet is compared).
    ex(&mut s, "edit.undo", json!({}));
    let doc = s.doc().unwrap().doc.clone();
    let back = ps::ensure_one(&doc, g).unwrap();
    assert_eq!(back.sheet, first.sheet);
    assert!(ps::lookup(&doc, doc.node(g).unwrap()).unwrap().1);
}

#[test]
fn a_copy_to_shape_target_moving_makes_the_stencil_stale() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0, WHITE);
    let target = rect(&mut s, 200.0, 0.0, 100.0, 100.0, BLACK);
    ex(&mut s, "modifier.add", json!({"ids": [a.0], "kind": "copyToShape", "props": {"target": target.0.to_string(), "count": 4}}));
    ex(&mut s, "modifier.add", json!({"ids": [a.0], "kind": "stencil"}));
    let doc = s.doc().unwrap().doc.clone();
    let first = ps::ensure_one(&doc, a).unwrap();
    assert!(first.info.paints >= 4, "{:?}", first.info);
    ex(&mut s, "object.move", json!({"ids": [target.0], "dx": 0, "dy": 50}));
    let doc = s.doc().unwrap().doc.clone();
    assert!(!ps::lookup(&doc, doc.node(a).unwrap()).unwrap().1, "the copies moved with their target");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn refresh_computes_in_the_background() {
    let mut s = session();
    let g = two_tone(&mut s, json!({"widthMm": 123}));
    let doc = s.doc().unwrap().doc.clone();
    assert!(ps::lookup(&doc, doc.node(g).unwrap()).is_none_or(|(_, fresh)| !fresh));
    assert!(ps::stencil_objects(&doc).contains(&g));
    ps::poll(&s, 0.0);
    assert!(ps::wait_idle(60.0));
    // Another session's job can still be running (tests share the scheduler): poll until fresh.
    for _ in 0..200 {
        if ps::lookup(&doc, doc.node(g).unwrap()).is_some_and(|(_, f)| f) {
            break;
        }
        ps::refresh(&doc, 0.0);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ps::lookup(&doc, doc.node(g).unwrap()).unwrap().1);
}

#[test]
fn a_stencil_without_a_size_says_so() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, BLACK);
    ex(&mut s, "modifier.add", json!({"ids": [a.0], "kind": "stencil"}));
    // Flatten it to nothing wide.
    s.edit("Flatten", |d, _| {
        let n = d.node_mut(a).unwrap();
        if let NodeKind::Path { path, .. } = &mut n.kind {
            *path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::kurbo::BezPath::from_svg("M0 0 L0 50 Z").unwrap());
        }
        Ok(())
    })
    .unwrap();
    let i = ex(&mut s, "modifier.stencilInfo", json!({"id": a.0}));
    assert_eq!(i["stencil"], Value::Null, "{i}");
    assert!(s.execute("modifier.stencilExport", &json!({"id": a.0, "dryRun": true})).is_err());
}

/// The test image: dark ground, a light ring holding a dark island, a light blob, pale specks.
fn ring_image() -> vectorcraft_trace::Raster {
    vectorcraft_trace::Raster::from_fn(160, 120, |x, y| {
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        let d1 = (fx - 50.0).hypot(fy - 60.0);
        let d2 = (fx - 120.0).hypot(fy - 50.0);
        if (18.0..34.0).contains(&d1) || d2 < 20.0 { [240, 220, 90, 255] } else { [30, 30, 40, 255] }
    })
}

fn add_image(s: &mut Session, r: &vectorcraft_trace::Raster) -> NodeId {
    let png = r.encode_png();
    let (w, h) = (r.width, r.height);
    s.edit("img", |d, sel| {
        d.images.insert("img1".into(), vectorcraft_doc::ImageBlob::new("image/png", png));
        let id = d.alloc_id();
        let l = d.default_layer();
        d.insert(
            l,
            0,
            Node::new(
                id,
                NodeKind::Image(Box::new(vectorcraft_doc::ImageObject {
                    key: "img1".into(),
                    width: w,
                    height: h,
                    xf: Affine::translate((10.0, 10.0)) * Affine::scale(2.0),
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                })),
            ),
        )?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn a_stencil_on_traced_paths_gets_a_sheet() {
    let mut s = session();
    add_image(&mut s, &ring_image());
    // Image Trace (expanded: a group of filled paths), then a stencil on the group.
    let r = ex(&mut s, "imageTrace.makeAndExpand", json!({"preset": "3 Colors", "params": {"mode": "color", "palette": "limited", "colors": 2}}));
    let traced = s.doc().unwrap().selection.objects.iter().copied().next().or_else(|| r["id"].as_u64().map(NodeId)).unwrap();
    let n = node(&s, traced);
    assert!(n.children().is_some_and(|c| c.len() >= 2), "a group of traced paths: {r}");
    let made = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [traced.0], "props": {"widthMm": 200}}));
    let g = id_of(&made["id"]);
    let i = ex(&mut s, "modifier.stencilInfo", json!({"id": g.0}));
    assert_eq!(i["stencil"], "m1", "{i}");
    assert_eq!(i["colours"].as_array().unwrap().len(), 2, "{i}");
    assert!(i["stats"]["cutPieces"].as_u64().unwrap() >= 2, "{i}");
    assert!(i["stats"]["islands"].as_u64().unwrap() >= 1, "the ring's dark middle is an island: {i}");
    assert_eq!(i["stats"]["untied"], 0, "{i}");
    let doc = s.doc().unwrap().doc.clone();
    let sheet = ps::stencil_sheet(&doc, doc.node(g).unwrap()).unwrap();
    assert!(sheet.cut.area().abs() > 0.0 && sheet.paper.area().abs() > 0.0);
    // And it exports.
    let e = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "dryRun": true}));
    assert_eq!(e["sheets"], 1, "{e}");
    assert!(e["files"][0]["contours"].as_u64().unwrap() > 2, "{e}");
}

#[test]
fn info_json_reads_non_finite_bounds_as_zero() {
    let info = ps::Info {
        islands: vec![
            vectorcraft_stencil::Island {
                bounds: vectorcraft_geom::Rect::new(40.0, 40.0, 60.0, 70.0),
                tied: 2,
                pattern_held: false,
                nested_depth: 0,
            },
            vectorcraft_stencil::Island {
                bounds: vectorcraft_geom::Rect::new(f64::NAN, 0.0, 1.0, 1.0),
                tied: 0,
                pattern_held: true,
                nested_depth: 1,
            },
        ],
        ..ps::Info::default()
    };
    let v = ps::info_json(&info);
    assert_eq!(
        v["islands"],
        json!([
            {"x": 40.0, "y": 40.0, "w": 20.0, "h": 30.0, "tied": 2, "patternHeld": false, "depth": 0},
            {"x": 0.0, "y": 0.0, "w": 0.0, "h": 1.0, "tied": 0, "patternHeld": true, "depth": 1},
        ])
    );
    assert!(serde_json::to_string(&v).is_ok());
}
