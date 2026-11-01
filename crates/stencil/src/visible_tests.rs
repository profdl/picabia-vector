use vectorcraft_geom::kurbo::{BezPath, Circle, Rect, Shape};

use crate::{Paint, visible_regions};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, colour: [f32; 4], order: u32) -> Paint {
    Paint { path: Rect::new(x0, y0, x1, y1).to_path(0.01), colour, order }
}

fn area_of(v: &crate::Visible, colour: [u8; 3]) -> f64 {
    v.regions.iter().find(|r| r.colour == colour).map(|r| r.area).unwrap_or(f64::NAN)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

#[test]
fn the_top_shape_hides_what_it_covers() {
    let v = visible_regions(&[rect(0.0, 0.0, 10.0, 10.0, RED, 0), rect(5.0, 0.0, 15.0, 10.0, BLUE, 1)], 0.01, 0.0);
    assert!(v.warnings.is_empty(), "{:?}", v.warnings);
    assert_eq!(v.regions.iter().map(|r| r.colour).collect::<Vec<_>>(), [[255, 0, 0], [0, 0, 255]], "bottom first");
    assert!(close(area_of(&v, [255, 0, 0]), 50.0));
    assert!(close(area_of(&v, [0, 0, 255]), 100.0));
    // The region's own path has the area it reports.
    for r in &v.regions {
        assert!(close(r.path.area(), r.area));
    }
}

#[test]
fn paint_order_not_input_order_decides() {
    let v = visible_regions(&[rect(5.0, 0.0, 15.0, 10.0, BLUE, 0), rect(0.0, 0.0, 10.0, 10.0, RED, 7)], 0.01, 0.0);
    assert!(close(area_of(&v, [255, 0, 0]), 100.0));
    assert!(close(area_of(&v, [0, 0, 255]), 50.0));
    // Equal orders stack in input order: the later one is on top.
    let v = visible_regions(&[rect(0.0, 0.0, 10.0, 10.0, RED, 3), rect(5.0, 0.0, 15.0, 10.0, BLUE, 3)], 0.01, 0.0);
    assert!(close(area_of(&v, [255, 0, 0]), 50.0));
}

#[test]
fn same_colour_runs_merge_and_split_colours_stay_one_region() {
    // Two overlapping reds in a row: their union, counted once.
    let v = visible_regions(&[rect(0.0, 0.0, 10.0, 10.0, RED, 0), rect(5.0, 0.0, 15.0, 10.0, RED, 1)], 0.01, 0.0);
    assert_eq!(v.regions.len(), 1);
    assert!(close(area_of(&v, [255, 0, 0]), 150.0));
    // Red, blue over it, red again on top: one red region (top red plus what shows of the bottom).
    let v =
        visible_regions(&[rect(0.0, 0.0, 30.0, 10.0, RED, 0), rect(10.0, 0.0, 30.0, 10.0, BLUE, 1), rect(20.0, 0.0, 30.0, 10.0, RED, 2)], 0.01, 0.0);
    assert_eq!(v.regions.len(), 2);
    assert!(close(area_of(&v, [255, 0, 0]), 200.0));
    assert!(close(area_of(&v, [0, 0, 255]), 100.0));
    let total: f64 = v.regions.iter().map(|r| r.area).sum();
    assert!(close(total, 300.0), "visible regions tile the union: {total}");
}

#[test]
fn a_covered_colour_has_no_area_and_holes_show_through() {
    let v = visible_regions(&[rect(2.0, 2.0, 4.0, 4.0, BLUE, 0), rect(0.0, 0.0, 10.0, 10.0, RED, 1)], 0.01, 0.0);
    assert_eq!(area_of(&v, [0, 0, 255]), 0.0);
    assert!(v.regions.iter().any(|r| r.colour == [0, 0, 255] && r.path.elements().is_empty()));
    // A red frame (hole wound either way, outer wound clockwise) over white: white shows in the hole.
    let mut frame = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.01).reverse_subpaths();
    frame.extend(Rect::new(3.0, 3.0, 7.0, 7.0).to_path(0.01).iter());
    let v = visible_regions(&[rect(0.0, 0.0, 10.0, 10.0, WHITE, 0), Paint { path: frame, colour: RED, order: 1 }], 0.01, 0.0);
    assert!(close(area_of(&v, [255, 255, 255]), 16.0));
    assert!(close(area_of(&v, [255, 0, 0]), 84.0));
}

#[test]
fn clockwise_and_self_overlapping_paints_union_correctly() {
    // A clockwise rect under a counter-clockwise one of the same colour: no cancelling.
    let cw = Paint { path: Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.01).reverse_subpaths(), colour: RED, order: 0 };
    let v = visible_regions(&[cw, rect(5.0, 0.0, 15.0, 10.0, RED, 1)], 0.01, 0.0);
    assert!(close(area_of(&v, [255, 0, 0]), 150.0));
    // Two subpaths of one paint wound the same way, overlapping (non-zero: their union).
    let mut p = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.01);
    p.extend(Rect::new(5.0, 0.0, 15.0, 10.0).to_path(0.01).iter());
    let v = visible_regions(&[Paint { path: p, colour: BLUE, order: 0 }], 0.01, 0.0);
    assert!(close(area_of(&v, [0, 0, 255]), 150.0));
}

#[test]
fn curves_flatten_within_the_tolerance() {
    let c = Paint { path: Circle::new((0.0, 0.0), 10.0).to_path(1e-3), colour: RED, order: 0 };
    let v = visible_regions(&[c], 0.01, 0.0);
    let a = area_of(&v, [255, 0, 0]);
    assert!((a - std::f64::consts::PI * 100.0).abs() < 2.0 * std::f64::consts::PI * 10.0 * 0.01, "{a}");
}

#[test]
fn bad_and_invisible_paints_are_left_out_with_warnings() {
    let mut nan = BezPath::new();
    nan.move_to((0.0, 0.0));
    nan.line_to((f64::NAN, 1.0));
    nan.line_to((1.0, 1.0));
    let paints = [
        rect(0.0, 0.0, 10.0, 10.0, RED, 0),
        rect(0.0, 0.0, 10.0, 10.0, [0.0, 0.0, 1.0, 0.0], 1),
        rect(0.0, 0.0, 5.0, 10.0, [0.0, 1.0, 0.0, 0.5], 2),
        Paint { path: nan, colour: BLUE, order: 3 },
        rect(0.0, 0.0, 1.0, 1.0, [f32::NAN, 0.0, 0.0, 1.0], 4),
        Paint { path: BezPath::new(), colour: BLUE, order: 5 },
    ];
    let v = visible_regions(&paints, 0.01, 0.0);
    assert_eq!(v.regions.len(), 2, "red and the translucent green: {:?}", v.regions.iter().map(|r| r.colour).collect::<Vec<_>>());
    assert!(close(area_of(&v, [255, 0, 0]), 50.0), "translucent green covers like an opaque one");
    let w = v.warnings.join("; ");
    assert!(w.contains("1 fully transparent"), "{w}");
    assert!(w.contains("1 translucent"), "{w}");
    assert!(w.contains("2 shapes with invalid"), "{w}");
}

#[test]
fn simplifying_drops_jitter_but_keeps_the_shape() {
    // A 100-unit square with 0.01-unit zigzag on every edge (traced-style).
    let mut p = BezPath::new();
    let mut pts = Vec::new();
    for i in 0..400 {
        let t = f64::from(i) / 100.0;
        let j = if i % 2 == 0 { 0.0 } else { 0.01 };
        let (x, y) = match i / 100 {
            0 => (t * 100.0, j),
            1 => (100.0 - j, (t - 1.0) * 100.0),
            2 => (100.0 - (t - 2.0) * 100.0, 100.0 - j),
            _ => (j, 100.0 - (t - 3.0) * 100.0),
        };
        pts.push((x, y));
    }
    p.move_to(pts[0]);
    for &q in &pts[1..] {
        p.line_to(q);
    }
    p.close_path();
    let raw = visible_regions(&[Paint { path: p.clone(), colour: RED, order: 0 }], 0.01, 0.0);
    let simple = visible_regions(&[Paint { path: p, colour: RED, order: 0 }], 0.01, 0.05);
    let (r, s) = (&raw.regions[0], &simple.regions[0]);
    assert!(s.path.elements().len() < 20 && r.path.elements().len() > 300, "{} → {}", r.path.elements().len(), s.path.elements().len());
    assert!((s.area - 10_000.0).abs() < 10.0, "{}", s.area);
}
