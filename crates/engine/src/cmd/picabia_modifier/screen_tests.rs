//! PICABIA: the `screen` row (PIC20.5c) through the commands: presets, the SVG export, Apply and
//! undo, an image's tone read through the document (and the image gone), image ids following
//! copies, the stencil and the laser cutting the dots.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Rect};

use super::tests::{data, err, ex, id_of, rect, session};
use crate::Session;

fn add(s: &mut Session, id: NodeId, props: Value) -> Value {
    ex(s, "modifier.add", json!({"ids": [id.0], "kind": "screen", "props": props}))
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

/// Every plain path of `d` as one compound path (the dots of a screened object).
fn all_paths(d: &Document) -> PathData {
    let mut sps = Vec::new();
    d.walk(|n| {
        if let NodeKind::Path { path, guide: false, .. } = &n.kind {
            sps.extend(path.subpaths.iter().cloned());
        }
    });
    PathData::new(sps)
}

/// The dots object `id` draws (its art's paths).
fn drawn(s: &Session, id: NodeId) -> PathData {
    let d = doc(s);
    let art = vectorcraft_render::picabia::copies_art(d, d.node(id).unwrap()).expect("drawn as its screen");
    PathData::new(art.children().unwrap().iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect())
}

fn area_in(p: &PathData, r: Rect) -> f64 {
    let clip = vectorcraft_geom::shapes::rectangle(r);
    let cut = vectorcraft_pathops::boolean(p, FillRule::NonZero, &clip, FillRule::NonZero, vectorcraft_pathops::BoolOp::Intersect);
    vectorcraft_pathops::area(&cut, FillRule::NonZero)
}

#[test]
fn presets_are_added_with_their_values_and_unknown_ones_refused() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "screen", "preset": "benDay"}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "screen", "preset": "lineFill", "props": {"angle": 0}}));
    let list = ex(&mut s, "modifier.list", json!({"id": r.0}));
    let rows = list["modifiers"].as_array().unwrap_or_else(|| panic!("{list}"));
    let p = |k: usize, key: &str| rows[k][key].clone();
    assert_eq!(
        (p(0, "spot"), p(0, "tone"), p(0, "amount"), p(0, "lpi"), p(0, "angle")),
        (json!("round"), json!("fixed"), json!(30.0), json!(10.0), json!(45.0))
    );
    assert_eq!((p(1, "spot"), p(1, "tone"), p(1, "angle")), (json!("line"), json!("fixed"), json!(0.0)), "props apply over the preset");
    let e = err(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "screen", "preset": "stipple"}));
    assert!(e.contains("benDay|lineFill"), "{e}");
    let e = err(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "trim", "preset": "benDay"}));
    assert!(e.contains("no preset"), "{e}");
}

/// A fixed 30% screen on the 100 pt square: the SVG writes the dots (about 30% of it, all inside
/// it), Apply bakes the same dots, undo restores the live row.
#[test]
fn a_screen_is_written_to_svg_and_apply_bakes_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r, json!({"tone": "fixed", "amount": 30, "lpi": 15}));
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "screen.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes()), "editingData": false}));
    let written = all_paths(doc(&t));
    assert!(written.subpaths.len() > 30, "dots: {}", written.subpaths.len());
    let share = vectorcraft_pathops::area(&written, FillRule::NonZero) / 10_000.0;
    assert!((share - 0.3).abs() < 0.03, "{share}");
    for a in written.subpaths.iter().flat_map(|sp| &sp.anchors) {
        assert!((-0.05..=100.05).contains(&a.p.x) && (-0.05..=100.05).contains(&a.p.y), "{:?} outside the square", a.p);
    }
    let before = data(&s, r);
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["applied"][0]["copies"], 1);
    assert_eq!(out["warnings"], json!([]));
    let baked = all_paths(doc(&s));
    assert_eq!(baked.subpaths.len(), written.subpaths.len());
    let baked_area = vectorcraft_pathops::area(&baked, FillRule::NonZero);
    assert!((baked_area / 10_000.0 - share).abs() < 1e-3, "baked as written: {baked_area}");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(data(&s, r), before);
}

/// A 200 pt square over an image black on its left half and white on its right: the dots ink the
/// left, not the right. The image deleted: the screen is skipped with a note; an id that names
/// nothing is refused when set.
#[test]
fn an_image_gives_the_tone_through_the_document() {
    let mut s = session();
    // 100 × 100 pixels at (10, 10), 2 pt a pixel: black for x < 50.
    let raster = vectorcraft_trace::Raster::from_fn(100, 100, |x, _| if x < 50 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
    let img = super::stencil_tests::add_image(&mut s, &raster);
    let sq = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 200, "height": 200}))["id"]);
    add(&mut s, sq, json!({"tone": "image", "image": img.0.to_string()}));
    let dots = drawn(&s, sq);
    let left = area_in(&dots, Rect::new(10.0, 10.0, 100.0, 210.0)) / (90.0 * 200.0);
    let right = area_in(&dots, Rect::new(120.0, 10.0, 210.0, 210.0)) / (90.0 * 200.0);
    assert!(left > 0.95 && right < 0.01, "{left} {right}");
    let d = doc(&s);
    assert!(vectorcraft_render::picabia::notes(d, d.node(sq).unwrap()).is_empty());
    // Image ids follow copies made together.
    let out = ex(&mut s, "edit.duplicate", json!({"ids": [sq.0, img.0]}));
    let copies: Vec<NodeId> = out["ids"].as_array().unwrap().iter().map(id_of).collect();
    let (src, image) = if data(&s, copies[0]).is_some() { (copies[0], copies[1]) } else { (copies[1], copies[0]) };
    let copied = data(&s, src).unwrap();
    assert!(copied.contains(&format!("\"image\":\"{}\"", image.0)), "{copied}");
    // The image gone: the square draws as it is, with the reason.
    ex(&mut s, "edit.clear", json!({"ids": [img.0]}));
    let d = doc(&s);
    let n = d.node(sq).unwrap();
    let notes = vectorcraft_render::picabia::notes(d, n);
    assert!(notes.iter().any(|m| m.contains("gone")), "{notes:?}");
    // Not an image: skipped, said.
    let other = rect(&mut s, 500.0);
    ex(&mut s, "modifier.set", json!({"ids": [sq.0], "index": 0, "props": {"image": other.0.to_string()}}));
    let d = doc(&s);
    let notes = vectorcraft_render::picabia::notes(d, d.node(sq).unwrap());
    assert!(notes.iter().any(|m| m.contains("isn't an image")), "{notes:?}");
    // An id that names nothing is refused when set, as a copy-to-shape target is.
    let e = err(&mut s, "modifier.set", json!({"ids": [sq.0], "index": 0, "props": {"image": "99999"}}));
    assert!(e.contains("gone"), "{e}");
    // The picker lists images only.
    let choices = super::image_choices(doc(&s), sq);
    assert!(choices.iter().all(|(id, _)| matches!(doc(&s).node(*id).unwrap().kind, NodeKind::Image(_))));
    assert_eq!(choices.len(), 1, "the copied image: {choices:?}");
}

/// The stencil cuts the dots: a fixed 30% screen above the stencil leaves each colour 30% of its
/// area.
#[test]
fn a_screen_above_a_stencil_cuts_its_dots() {
    let mut s = session();
    let g = super::stencil_tests::two_tone(&mut s, json!({}));
    let white = |s: &mut Session| {
        let i = ex(s, "modifier.stencilInfo", json!({"id": g.0}));
        i["colours"].as_array().unwrap().iter().find(|c| c["hex"] == "#ffffff").unwrap()["area"].as_f64().unwrap()
    };
    let whole = white(&mut s);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "screen", "index": 0, "props": {"tone": "fixed", "amount": 30, "lpi": 10}}));
    let dots = white(&mut s);
    assert!((dots / whole - 0.3).abs() < 0.04, "{dots} of {whole}");
}

/// The laser burns the dots' outlines: as long as the drawn dots' perimeters.
#[test]
fn the_laser_cuts_the_screened_design() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    let burn = |s: &mut Session| ex(s, "laser.gcode", json!({"ids": [r.0], "data": true}))["files"][0]["burnMm"].as_f64().unwrap();
    let square = burn(&mut s);
    add(&mut s, r, json!({"tone": "fixed", "amount": 30, "lpi": 10}));
    let screened = burn(&mut s);
    let dots = drawn(&s, r);
    let perimeter: f64 =
        dots.subpaths.iter().map(|sp| (0..sp.segment_count()).map(|i| kurbo::ParamCurveArclen::arclen(&sp.segment(i), 1e-6)).sum::<f64>()).sum();
    // Closed outlines are cut in as many passes as the square was (its 400 pt perimeter).
    let passes = (square / (400.0 * 25.4 / 72.0)).round();
    let mm = perimeter * 25.4 / 72.0 * passes;
    assert!(dots.subpaths.len() > 100, "{}", dots.subpaths.len());
    assert!((screened - mm).abs() < 0.02 * mm, "{screened} mm burnt for {mm} mm of dots (the square alone: {square})");
}
