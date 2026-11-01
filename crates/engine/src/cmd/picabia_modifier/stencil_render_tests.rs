//! PICABIA: stencil sheets on the canvas and in exports: the paper/cloth preview in place of the
//! design, bounds and clicks on the sheet, the tile output, and exports computing stale sheets
//! first. (Ported from picabia-craft's tests that waited on the renderer.)

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_doc::hit::{HitOptions, hit_test, marquee};
use vectorcraft_geom::{Point, Rect};

use super::stencil_tests::{BLACK, WHITE, rect, two_tone};
use super::tests::{ex, id_of, session};
use crate::Session;
use crate::picabia_stencil as ps;

/// Pixel `(x, y)` of `doc` drawn over `region` at 1 px a point, transparent where nothing is. The
/// sheets are computed just before: the store is keyed by object id and shared by the tests
/// running at once, whose documents reuse the same ids, so another test can replace this one's
/// result between a compute and a draw.
fn px(doc: &std::sync::Arc<vectorcraft_doc::Document>, region: Rect, x: f64, y: f64) -> [u8; 4] {
    ps::ensure(doc);
    let r = vectorcraft_render::Renderer::new().render_region(doc, region, 1.0, false);
    let i = (((y - region.y0) as u32 * r.width + (x - region.x0) as u32) * 4) as usize;
    [r.pixels[i], r.pixels[i + 1], r.pixels[i + 2], r.pixels[i + 3]]
}

fn doc(s: &Session) -> vectorcraft_doc::Document {
    (*s.doc().unwrap().doc).clone()
}

#[test]
fn the_canvas_draws_the_sheet_in_place_of_the_design() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let d = s.doc().unwrap().doc.clone();
    ps::ensure(&d);
    // The 200 × 100 design at 300 mm: 1.5 mm a unit, so the 15 mm margin is 10 units.
    let world = vectorcraft_render::painted_bounds(&d, d.node(g).unwrap()).unwrap();
    assert_eq!(world, Rect::new(-10.0, -10.0, 210.0, 110.0));
    // Margin (not by a registration hole): paper, kakishibu brown.
    assert_eq!(px(&d, world, -5.0, 50.0), [0xc8, 0xa1, 0x65, 0xff]);
    // Where the black square was: paper too; the white square is cut, the canvas shows through.
    assert_eq!(px(&d, world, 50.0, 50.0), [0xc8, 0xa1, 0x65, 0xff]);
    assert_eq!(px(&d, world, 150.0, 50.0)[3], 0, "cut out");
    // The cloth preview: cloth with the paste through the cut.
    ex(&mut s, "modifier.set", json!({"ids": [g.0], "index": 0, "props": {"preview": "cloth"}}));
    let d = s.doc().unwrap().doc.clone();
    ps::ensure(&d);
    assert_eq!(px(&d, world, -5.0, 50.0), [0xf4, 0xef, 0xe6, 0xff], "cloth");
    assert_eq!(px(&d, world, 150.0, 50.0), [0x1f, 0x3a, 0x8a, 0xff], "paste");
}

#[test]
fn the_sheet_counts_for_bounds_and_hit_testing() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    // Before a sheet is computed: its frame already counts.
    let d = doc(&s);
    let leaf = |d: &vectorcraft_doc::Document, x: f64, y: f64| hit_test(d, Point::new(x, y), HitOptions::default()).map(|h| h.leaf);
    assert_eq!(leaf(&d, -5.0, 50.0), Some(g), "on the margin, outside the design");
    assert_eq!(leaf(&d, -50.0, -50.0), None);
    ps::ensure(&s.doc().unwrap().doc);
    let d = doc(&s);
    assert_eq!(leaf(&d, -5.0, 50.0), Some(g));
    assert_eq!(marquee(&d, Rect::new(-8.0, 40.0, -2.0, 60.0), None, false), [g], "a marquee over the margin");
}

#[test]
fn exports_compute_stale_stencils_first() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    // Moved where no other test's stencil is: the store keeps one result per object id and
    // geometry, shared by the tests running at once (whose documents reuse the same ids).
    ex(&mut s, "object.move", json!({"ids": [g.0], "dx": 13, "dy": 0}));
    let d = s.doc().unwrap().doc.clone();
    assert!(ps::lookup(&d, d.node(g).unwrap()).is_none_or(|(_, f)| !f));
    let out = ex(&mut s, "document.serialize", json!({"format": "png"}));
    assert!(out["dataBase64"].as_str().is_some_and(|d| !d.is_empty()));
    let d = s.doc().unwrap().doc.clone();
    assert!(ps::lookup(&d, d.node(g).unwrap()).is_some_and(|(_, f)| f), "the export brought it up to date");
}

#[test]
fn svg_and_pdf_exports_draw_the_sheet() {
    let mut s = session();
    let _g = two_tone(&mut s, json!({}));
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "useArtboards": false}))["text"].as_str().unwrap().to_string();
    assert!(svg.contains("#c8a165") && svg.contains("evenodd"), "{svg}");
    assert!(!svg.contains("#000000"), "the design itself isn't written: {svg}");
    // The export covers the sheet: 200 wide plus 10 units each side.
    assert!(svg.contains("width=\"220\""), "{svg}");
    let out = ex(&mut s, "document.serialize", json!({"format": "pdf"}));
    assert!(out["dataBase64"].as_str().is_some_and(|d| !d.is_empty()));
}

/// A 20-unit corner motif, gridded 3 × 3 with no gap, ending in a stencil row with `props`.
fn corner_motif(s: &mut Session, props: Value) -> NodeId {
    let mut ids = vec![rect(s, 0.0, 0.0, 20.0, 20.0, BLACK).0];
    for (x, y) in [(0.0, 0.0), (14.0, 0.0), (0.0, 14.0), (14.0, 14.0)] {
        ids.push(rect(s, x, y, 6.0, 6.0, WHITE).0);
    }
    for (x, y) in [(0.0, 0.0), (18.0, 0.0), (0.0, 18.0), (18.0, 18.0)] {
        ids.push(rect(s, x, y, 2.0, 2.0, BLACK).0);
    }
    ex(s, "select.set", json!({"ids": ids}));
    let g = id_of(&ex(s, "object.group", json!({}))["id"]);
    ex(s, "modifier.add", json!({"ids": [g.0], "kind": "gridArray", "props": {"rows": 3, "cols": 3, "gapX": 0, "gapY": 0}}));
    ex(s, "modifier.add", json!({"ids": [g.0], "kind": "stencil", "props": props}));
    g
}

#[test]
fn tile_output_places_the_sheet_on_the_repeat_cell() {
    let mut s = session();
    let g = corner_motif(&mut s, json!({"output": "tile", "tieSeed": 4242}));
    let d = s.doc().unwrap().doc.clone();
    let n = d.node(g).unwrap();
    // 60 units across at 300 mm: 5 mm a unit, the 15 mm margin is 3 units. Known before compute.
    let frame = vectorcraft_stencil::store::frame(&d, n).unwrap();
    let cell = frame.sheet;
    assert!((cell.width() - 26.0).abs() < 1e-9 && (cell.height() - 26.0).abs() < 1e-9, "{cell:?}");
    assert!(frame.cell.is_some());
    let sheet = ps::compute_now(&d, g).unwrap();
    let close = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 1e-6);
    assert!(close(sheet.sheet, frame.sheet), "what is cut is where the frame put it");
    // Bounds and hit testing follow the tile.
    let world = vectorcraft_render::painted_bounds(&d, n).unwrap();
    assert!(close(world, frame.sheet), "{world:?}");
    let leaf = |x: f64, y: f64| hit_test(&d, Point::new(x, y), HitOptions::default()).map(|h| h.leaf);
    assert_eq!(leaf(frame.sheet.x0 + 1.0, frame.sheet.center().y), Some(g), "on the tile's margin");
    assert_eq!(leaf(-2.0, -2.0), None, "where the whole sheet's margin would be");
}

#[test]
fn the_tile_frame_matches_what_the_stencil_cuts() {
    let cases = [
        json!({"kind": "gridArray", "props": {"rows": 3, "cols": 4, "gapX": 0, "gapY": 0}}),
        json!({"kind": "gridArray", "props": {"rows": 1, "cols": 4, "gapX": 0, "gapY": 0}}),
        json!({"kind": "gridArray", "props": {"rows": 3, "cols": 1, "gapX": 0, "gapY": 0}}),
        json!({"kind": "wallpaper", "props": {"group": "p1", "cols": 3, "rows": 3, "cellWidth": 20, "cellHeight": 20, "angle": 60}}),
        json!({"kind": "wallpaper", "props": {"group": "p6", "cols": 2, "rows": 2, "cellWidth": 25, "cellHeight": 25}}),
    ];
    for (k, row) in cases.iter().enumerate() {
        for rotate in [false, true] {
            let mut s = session();
            let g = corner_motif(&mut s, json!({"output": "tile"}));
            // The case's lattice in place of the grid.
            ex(&mut s, "modifier.remove", json!({"ids": [g.0], "index": 0}));
            let mut add = row.clone();
            add["ids"] = json!([g.0]);
            add["index"] = json!(0);
            ex(&mut s, "modifier.add", add);
            if rotate {
                ex(&mut s, "object.rotate", json!({"ids": [g.0], "angle": 17}));
            }
            let d = s.doc().unwrap().doc.clone();
            let n = d.node(g).unwrap();
            let frame = vectorcraft_stencil::store::frame(&d, n).unwrap();
            assert!(frame.repeat.is_some(), "case {k}");
            let sheet = ps::compute_now(&d, g).unwrap();
            let close = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 1e-6);
            assert!(
                close(sheet.sheet, frame.sheet),
                "case {k} rotate {rotate}: cut {:?}, frame {:?}; {:?}",
                sheet.sheet,
                frame.sheet,
                sheet.info.warnings
            );
            assert!(sheet.info.tile_mm.is_some(), "case {k}: {:?}", sheet.info.warnings);
        }
    }
}

#[test]
fn svg_and_pdf_exports_write_the_tile_where_it_is_cut() {
    let mut s = session();
    let ids: Vec<u64> = [(0.0, 0.0, 20.0, BLACK), (14.0, 14.0, 6.0, WHITE), (18.0, 18.0, 2.0, BLACK)]
        .into_iter()
        .map(|(x, y, w, hex)| rect(&mut s, x, y, w, w, hex).0)
        .collect();
    let g = id_of(&ex(&mut s, "modifier.stencilFromSelection", json!({"ids": ids, "props": {"output": "tile", "tieSeed": 99}}))["id"]);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "gridArray", "props": {"rows": 3, "cols": 3, "gapX": 0, "gapY": 0}, "index": 0}));
    let i = ex(&mut s, "modifier.stencilInfo", json!({"id": g.0}));
    assert_eq!(i["tile"], json!({"w": 130.0, "h": 130.0}), "{i}");
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "useArtboards": false}))["text"].as_str().unwrap().to_string();
    // The tile: the 20-unit cell plus a 3-unit margin each side, not the 66-unit sheet.
    assert!(svg.contains("width=\"26\"") && svg.contains("height=\"26\""), "{svg}");
    assert!(svg.contains("#c8a165") && !svg.contains("#000000"), "{svg}");
    let out = ex(&mut s, "document.serialize", json!({"format": "pdf"}));
    assert!(out["dataBase64"].as_str().is_some_and(|d| !d.is_empty()));
}

#[test]
fn tile_output_exports_the_tile_for_the_laser() {
    let mut s = session();
    let ids: Vec<u64> = [(0.0, 0.0, 20.0, BLACK), (14.0, 14.0, 6.0, WHITE), (18.0, 18.0, 2.0, BLACK)]
        .into_iter()
        .map(|(x, y, w, hex)| rect(&mut s, x, y, w, w, hex).0)
        .collect();
    let g = id_of(&ex(&mut s, "modifier.stencilFromSelection", json!({"ids": ids, "props": {"output": "tile", "tieSeed": 99}}))["id"]);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "gridArray", "props": {"rows": 3, "cols": 3, "gapX": 0, "gapY": 0}, "index": 0}));
    let r = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "data": true}));
    assert_eq!(r["sheets"], 1, "{r}");
    // The 130 mm tile (not the 330 mm sheet) plus paper's kerf.
    let close = |v: &Value, want: f64| v.as_f64().is_some_and(|x| (x - want).abs() < 1e-6);
    assert!(close(&r["files"][0]["wMm"], 130.1) && close(&r["files"][0]["hMm"], 130.1), "{r}");
}
