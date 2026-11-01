//! PICABIA: modifier copies in exports: SVG, PDF, EPS, DXF, EMF and raster formats write every
//! copy, the written copies carry no stack, Preserve Editing keeps the live stack, and the formats
//! that bake say so.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};

use super::tests::{arrayed, data, ex};
use crate::Session;

/// The paths of `d` (leaves drawn: paths and compound paths) and how many carry a stack.
fn paths(d: &Document) -> (usize, usize) {
    let (mut n, mut stacked) = (0, 0);
    d.walk(|x| {
        if matches!(x.kind, NodeKind::Path { guide: false, .. }) {
            n += 1;
        }
        if vectorcraft_modifiers::has_modifiers(x) {
            stacked += 1;
        }
    });
    (n, stacked)
}

fn reopen(name: &str, data: &Value) -> Session {
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": name, "dataBase64": data, "editingData": false}));
    t
}

fn warned(out: &Value) -> bool {
    out["warnings"].as_array().is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("modifier copies"))))
}

#[test]
fn svg_writes_every_copy_without_the_stack_and_preserve_editing_keeps_it_live() {
    let (mut s, r) = arrayed();
    let out = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}));
    let svg = out["text"].as_str().unwrap().to_string();
    assert!(!svg.contains(vectorcraft_modifiers::PLUGIN_KEY), "the copies carry no stack: {svg}");
    assert!(warned(&out), "{out}");
    let t = reopen("copies.svg", &json!(vectorcraft_format::base64_encode(svg.as_bytes())));
    assert_eq!(paths(&t.doc().unwrap().doc), (5, 0), "five plain copies, not copied again");
    // With Preserve Editing the file reopens as the document it was: one object, live.
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": true}))["text"].as_str().unwrap().to_string();
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "live.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}));
    assert_eq!(paths(&t.doc().unwrap().doc), (1, 1));
    assert_eq!(ex(&mut t, "modifier.list", json!({"id": r.0}))["instances"], 5);
    // The document exported keeps its stack.
    assert!(data(&s, r).is_some());
}

#[test]
fn pdf_eps_dxf_and_metafiles_write_every_copy() {
    let (mut s, _) = arrayed();
    for (format, name) in [("pdf", "copies.pdf"), ("dxf", "copies.dxf"), ("emf", "copies.emf")] {
        let out = ex(&mut s, "document.serialize", json!({"format": format, "preserveEditing": false}));
        assert!(warned(&out), "{format}: {out}");
        let t = reopen(name, &out["dataBase64"]);
        let (n, stacked) = paths(&t.doc().unwrap().doc);
        assert!(n >= 5, "{format}: {n} paths");
        assert_eq!(stacked, 0, "{format}");
    }
    // An EPS VectorCraft wrote carries the document itself: it reopens live, and its PostScript
    // draws the copies for other software (five filled rectangles).
    let out = ex(&mut s, "document.serialize", json!({"format": "eps"}));
    assert!(warned(&out), "{out}");
    let eps = vectorcraft_format::base64_decode(out["dataBase64"].as_str().unwrap()).unwrap();
    let t = reopen("live.eps", &out["dataBase64"]);
    assert_eq!(paths(&t.doc().unwrap().doc), (1, 1), "the EPS restores the live stack");
    let ps = String::from_utf8_lossy(&eps);
    let filled = ps.lines().filter(|l| l.trim_end().ends_with(" f") || l.trim() == "f").count();
    assert!(filled >= 5, "{filled} fills in the PostScript");
}

#[test]
fn raster_exports_draw_every_copy_and_the_native_file_bakes_nothing() {
    let (mut s, _) = arrayed();
    // The rectangle (0,0 100×100) and its copies 120 apart: 0..580 across.
    let out = ex(&mut s, "document.serialize", json!({"format": "png", "useArtboards": false}));
    assert!(!warned(&out));
    let png = vectorcraft_format::base64_decode(out["dataBase64"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert!(img.width() >= 580, "the export covers every copy: {}", img.width());
    let at = |x: u32| img.get_pixel(x, 50).0;
    assert_eq!(at(img.width() - 50)[3], 255, "the last copy is drawn");
    assert_eq!(at(110)[3], 0, "between copies is empty");
    let out = ex(&mut s, "document.serialize", json!({"format": "vectorcraft"}));
    assert!(!warned(&out));
}
