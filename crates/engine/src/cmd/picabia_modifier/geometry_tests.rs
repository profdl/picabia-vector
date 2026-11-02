//! PICABIA: geometry rows (`resample`, `trim`, `tidy`) through the commands: exports write the
//! reshaped paths, Apply bakes them, the panel's list reports them, undo restores.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::SubPath;

use super::tests::{ex, rect, session};
use crate::Session;

/// The subpaths of the plain paths of `d`, in tree order.
fn subpaths(d: &Document) -> Vec<SubPath> {
    let mut out = Vec::new();
    d.walk(|n| {
        if let NodeKind::Path { path, guide: false, .. } = &n.kind {
            out.extend(path.subpaths.iter().cloned());
        }
    });
    out
}

fn length(sp: &SubPath) -> f64 {
    sp.anchor_arc_positions().last().copied().unwrap_or(0.0) + if sp.closed { closing(sp) } else { 0.0 }
}

fn closing(sp: &SubPath) -> f64 {
    let n = sp.anchors.len();
    if n < 2 { 0.0 } else { kurbo::ParamCurveArclen::arclen(&sp.segment(n - 1), 1e-6) }
}

fn reopen_svg(svg: &str) -> Session {
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "shaped.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes()), "editingData": false}));
    t
}

fn add(s: &mut Session, id: u64, kind: &str, props: Value) {
    ex(s, "modifier.add", json!({"ids": [id], "kind": kind, "props": props}));
}

#[test]
fn resample_is_written_to_svg_and_apply_bakes_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "resample", json!({"mode": "count", "count": 8}));
    let list = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(list["instances"], 1, "{list}");
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
    let t = reopen_svg(&svg);
    let sps = subpaths(&t.doc().unwrap().doc);
    assert_eq!(sps.len(), 1);
    assert_eq!(sps[0].anchors.len(), 8, "the resampled square: {svg}");
    // Apply: one reshaped copy in a group, no stack left; undo puts the live row back.
    let before = super::tests::data(&s, r);
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["applied"][0]["copies"], 1);
    let d = &s.doc().unwrap().doc;
    let sps = subpaths(d);
    assert_eq!(sps.len(), 1);
    assert_eq!(sps[0].anchors.len(), 8);
    let mut stacked = 0;
    d.walk(|n| stacked += usize::from(vectorcraft_modifiers::has_modifiers(n)));
    assert_eq!(stacked, 0);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(super::tests::data(&s, r), before);
}

#[test]
fn trim_below_an_array_trims_every_copy_and_above_trims_the_object_once() {
    for (rows, label) in [
        (vec![("linearArray", json!({"count": 3, "offsetX": 120})), ("trim", json!({"start": 0, "end": 0.5}))], "below"),
        (vec![("trim", json!({"start": 0, "end": 0.5})), ("linearArray", json!({"count": 3, "offsetX": 120}))], "above"),
    ] {
        let mut s = session();
        let r = rect(&mut s, 0.0);
        for (kind, props) in rows {
            add(&mut s, r.0, kind, props);
        }
        ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
        let sps = subpaths(&s.doc().unwrap().doc);
        assert_eq!(sps.len(), 3, "{label}");
        for (k, sp) in sps.iter().enumerate() {
            assert!(!sp.closed, "{label}");
            assert!((length(sp) - 200.0).abs() < 1e-6, "{label}: half the square, {}", length(sp));
            // Top-left, top-right, bottom-right of copy k.
            let x0 = 120.0 * k as f64;
            assert!((sp.anchors[0].p.x - x0).abs() < 1e-6 && (sp.anchors[2].p.x - (x0 + 100.0)).abs() < 1e-6, "{label}");
        }
    }
}

#[test]
fn tidy_closes_an_open_path_in_pdf_and_dxf_too() {
    let mut s = session();
    let p = super::tests::id_of(&ex(&mut s, "path.create", json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]}))["id"]);
    add(&mut s, p.0, "tidy", json!({}));
    for format in ["svg", "pdf", "dxf"] {
        let out = ex(&mut s, "document.serialize", json!({"format": format, "preserveEditing": false}));
        let data = out["dataBase64"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| vectorcraft_format::base64_encode(out["text"].as_str().unwrap().as_bytes()));
        let mut t = Session::new();
        ex(&mut t, "document.open", json!({"name": format!("closed.{format}"), "dataBase64": data, "editingData": false}));
        let sps = subpaths(&t.doc().unwrap().doc);
        assert!(
            sps.iter().any(|sp| sp.closed || sp.anchors.first().zip(sp.anchors.last()).is_some_and(|(a, b)| a.p.distance(b.p) < 1e-3)),
            "{format}: closed: {sps:?}"
        );
    }
}

#[test]
fn geometry_rows_are_set_and_listed_like_any_row() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "trim", json!({}));
    ex(&mut s, "modifier.set", json!({"ids": [r.0], "index": 0, "props": {"start": 0.25, "end": 2}}));
    let list = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(list["modifiers"][0]["kind"], "trim");
    assert_eq!(list["modifiers"][0]["label"], "Trim");
    assert_eq!(list["modifiers"][0]["start"], 0.25);
    // Stored as given; clamped (with a warning) when evaluated.
    assert!(list["warnings"].as_array().unwrap().iter().any(|w| w["message"].as_str().unwrap().contains("end")), "{list}");
    assert!(s.execute("modifier.set", &json!({"ids": [r.0], "index": 0, "props": {"start": "NaN"}})).is_err());
    assert!(s.execute("modifier.add", &json!({"ids": [r.0], "kind": "resample", "props": {"mode": "wobble"}})).is_err());
}

/// A row skipped over the anchor cap isn't baked silently: Apply says so.
#[test]
fn apply_warns_when_a_row_was_skipped_over_the_cap() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "linearArray", json!({"count": 1000, "offsetX": 1}));
    add(&mut s, r.0, "resample", json!({"count": 1024}));
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    let w = out["warnings"].as_array().unwrap();
    assert!(w.iter().any(|w| w.as_str().unwrap().contains("anchors")), "{out}");
    // Within the cap: no warnings.
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "resample", json!({"count": 12}));
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["warnings"], json!([]));
}

/// A geometry row above a stencil reshapes what it cuts: a trim keeping half of each square's
/// outline leaves each filled as the triangle under it, half the area.
#[test]
fn a_trim_above_a_stencil_reshapes_what_it_cuts() {
    let mut s = session();
    let g = super::stencil_tests::two_tone(&mut s, json!({}));
    let white = |s: &mut Session| {
        let i = ex(s, "modifier.stencilInfo", json!({"id": g.0}));
        i["colours"].as_array().unwrap().iter().find(|c| c["hex"] == "#ffffff").unwrap()["area"].as_f64().unwrap()
    };
    let whole = white(&mut s);
    assert!((whole - 150.0 * 150.0).abs() < 1.0, "{whole}");
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "trim", "index": 0, "props": {"start": 0, "end": 0.5}}));
    let half = white(&mut s);
    assert!((half - whole / 2.0).abs() < 1.0, "{half} vs {whole}");
}

#[test]
fn the_laser_cuts_the_trimmed_design() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    let gcode = |s: &mut Session| {
        let out = ex(s, "laser.gcode", json!({"ids": [r.0], "data": true}));
        out["files"][0]["burnMm"].as_f64().unwrap()
    };
    let whole = gcode(&mut s);
    add(&mut s, r.0, "trim", json!({"start": 0, "end": 0.5}));
    let half = gcode(&mut s);
    // The open half (200 points) is burnt once, as any open path; the closed square was more.
    let mm = 200.0 * 25.4 / 72.0;
    assert!((half - mm).abs() < 0.01 * mm && half < whole, "{half} vs {whole}");
}

/// PIC20.4: a displace below an array moves each copy's corners differently; the SVG writes the
/// moved corners, Apply bakes the same ones, undo restores the live row. Radial with a progressive
/// ramp: each copy's shape depends on its index (its phase and its share), so a renderer and an
/// Apply that numbered the copies differently would disagree here.
#[test]
fn displace_below_an_array_is_written_to_svg_and_apply_bakes_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    add(&mut s, r.0, "linearArray", json!({"count": 3, "offsetX": 150}));
    add(&mut s, r.0, "displace", json!({"mode": "radial", "amplitude": 100, "rippleCount": 3, "progressiveReverse": true}));
    let corners = |sps: &[SubPath]| -> Vec<Vec<(f64, f64)>> {
        sps.iter().enumerate().map(|(k, sp)| sp.anchors.iter().map(|a| (a.p.x - 150.0 * k as f64, a.p.y)).collect()).collect()
    };
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
    let written = corners(&subpaths(&reopen_svg(&svg).doc().unwrap().doc));
    assert_eq!(written.len(), 3, "{svg}");
    let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    for (k, c) in written.iter().enumerate() {
        assert_eq!(c.len(), 4);
        let moved = c.iter().zip(&square).any(|(p, q)| (p.0 - q.0).abs() > 0.5 || (p.1 - q.1).abs() > 0.5);
        // Reversed ramp: copy 0 all of it, copy 1 half, the last none.
        assert_eq!(moved, k < 2, "copy {k}: {c:?}");
        // A quarter of the corner's distance from the centre (71 pt), at most.
        assert!(c.iter().zip(&square).all(|(p, q)| (p.0 - q.0).abs() <= 18.0 && (p.1 - q.1).abs() <= 18.0), "within the amplitude: {c:?}");
    }
    assert_ne!(written[0], written[1], "each copy its own phase and share");
    let before = super::tests::data(&s, r);
    let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(out["applied"][0]["copies"], 3);
    assert_eq!(out["warnings"], json!([]));
    let baked = corners(&subpaths(&s.doc().unwrap().doc));
    for (b, w) in baked.iter().zip(&written) {
        for (p, q) in b.iter().zip(w) {
            assert!((p.0 - q.0).abs() < 1e-2 && (p.1 - q.1).abs() < 1e-2, "baked as written: {b:?} vs {w:?}");
        }
    }
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(super::tests::data(&s, r), before);
}

/// PIC20.4c: the wave, jitter and fractal patterns below an array: the SVG writes the shaped
/// copies (a fractal's added anchors too), Apply bakes the same ones, undo restores the live row.
#[test]
fn displace_patterns_are_written_to_svg_and_apply_bakes_them() {
    for (props, anchors) in [
        (json!({"mode": "wave", "amplitude": 20, "direction": "normal"}), 4),
        (json!({"mode": "jitter", "amplitude": 20, "jitterMin": -1}), 4),
        (json!({"mode": "fractal", "amplitude": 30, "steps": 2}), 16),
    ] {
        let mut s = session();
        let r = rect(&mut s, 0.0);
        add(&mut s, r.0, "linearArray", json!({"count": 3, "offsetX": 150}));
        add(&mut s, r.0, "displace", props.clone());
        let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
        let written = subpaths(&reopen_svg(&svg).doc().unwrap().doc);
        assert_eq!(written.len(), 3, "{props}: {svg}");
        // The ramp: copy 0 is the square, the others shaped.
        assert_eq!(written[0].anchors.len(), 4, "{props}");
        assert_eq!(written[2].anchors.len(), anchors, "{props}");
        let before = super::tests::data(&s, r);
        let out = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
        assert_eq!(out["applied"][0]["copies"], 3, "{props}");
        assert_eq!(out["warnings"], json!([]), "{props}");
        let baked = subpaths(&s.doc().unwrap().doc);
        for (b, w) in baked.iter().zip(&written) {
            assert_eq!(b.anchors.len(), w.anchors.len(), "{props}");
            for (p, q) in b.anchors.iter().zip(&w.anchors) {
                assert!(p.p.distance(q.p) < 1e-2, "{props}: baked as written");
            }
        }
        ex(&mut s, "edit.undo", json!({}));
        assert_eq!(super::tests::data(&s, r), before, "{props}");
    }
}
