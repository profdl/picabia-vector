//! PICABIA: the `deform` geometry row through the commands: SVG export writes the deformed paths,
//! Apply bakes them (ramped across the copies), undo restores, params are clamped with a warning.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::{Rect, SubPath};

use super::tests::{ex, rect, session};
use crate::Session;

fn subpaths(d: &Document) -> Vec<SubPath> {
    let mut out = Vec::new();
    d.walk(|n| {
        if let NodeKind::Path { path, guide: false, .. } = &n.kind {
            out.extend(path.subpaths.iter().cloned());
        }
    });
    out
}

fn bounds(sp: &SubPath) -> Rect {
    vectorcraft_geom::PathData::new(vec![sp.clone()]).bounds().unwrap()
}

fn add(s: &mut Session, id: u64, kind: &str, props: Value) {
    ex(s, "modifier.add", json!({"ids": [id], "kind": kind, "props": props}));
}

/// A twist below a three-copy array: the first copy as it was, the others twisted more and more;
/// the SVG and Apply have the same paths, and undo puts the live rows back.
#[test]
fn a_ramped_twist_is_exported_and_applied_and_undone() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "linearArray", json!({"count": 3, "offsetX": 300}));
    add(&mut s, r.0, "deform", json!({"mode": "twist", "angle": 90}));
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "d.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes()), "editingData": false}));
    let exported = subpaths(&t.doc().unwrap().doc);
    assert_eq!(exported.len(), 3, "{svg}");
    assert_eq!(exported[0].anchors.len(), 4, "the first copy untouched");
    // A square twisted: its box grows as its ends turn (half the angle on the middle copy).
    let h: Vec<f64> = exported.iter().map(|sp| bounds(sp).height()).collect();
    assert!((h[0] - 100.0).abs() < 1e-3 && h[1] > h[0] + 1.0 && h[2] > h[1] + 1.0, "{h:?}");
    let before = super::tests::data(&s, r);
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["applied"][0]["copies"], 3);
    assert_eq!(out["warnings"], json!([]));
    let applied = subpaths(&s.doc().unwrap().doc);
    assert_eq!(applied.len(), 3);
    for (a, e) in applied.iter().zip(&exported) {
        assert_eq!(a.anchors.len(), e.anchors.len());
        let (ba, be) = (bounds(a), bounds(e));
        assert!((ba.x0 - be.x0).abs() < 0.01 && (ba.y1 - be.y1).abs() < 0.01, "{ba:?} vs {be:?}");
    }
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(super::tests::data(&s, r), before);
    assert_eq!(subpaths(&s.doc().unwrap().doc)[0].anchors.len(), 4, "the square again");
}

#[test]
fn deform_params_are_set_listed_and_clamped() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "deform", json!({"mode": "bulge"}));
    ex(&mut s, "modifier.set", json!({"ids": [r.0], "index": 0, "props": {"strength": 9, "falloff": "smooth"}}));
    let list = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(list["modifiers"][0]["kind"], "deform");
    assert_eq!(list["modifiers"][0]["label"], "Deform");
    assert_eq!(list["modifiers"][0]["strength"], 9.0);
    assert!(list["warnings"].as_array().unwrap().iter().any(|w| w["message"].as_str().unwrap().contains("strength")), "{list}");
    assert!(s.execute("modifier.set", &json!({"ids": [r.0], "index": 0, "props": {"mode": "melt"}})).is_err());
    assert!(s.execute("modifier.set", &json!({"ids": [r.0], "index": 0, "props": {"radius": "big"}})).is_err());
    // A bulge alone: one reshaped copy, wider than the square.
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["applied"][0]["copies"], 1);
    let sps = subpaths(&s.doc().unwrap().doc);
    assert!(bounds(&sps[0]).width() > 100.0 + 1.0, "{:?}", bounds(&sps[0]));
}
