//! PICABIA: stencil sheets on the canvas: the paper/cloth preview in place of the design, bounds
//! and clicks on the sheet. (Ported from picabia-craft's tests that waited on the renderer.)

use serde_json::json;
use vectorcraft_doc::hit::{HitOptions, hit_test, marquee};
use vectorcraft_geom::{Point, Rect};

use super::stencil_tests::two_tone;
use super::tests::{ex, session};
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
