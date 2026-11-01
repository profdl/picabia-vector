//! Gathering a design from VectorCraft objects: paint order, fill rules, strokes, clip groups,
//! copies, modifier stacks inside, what is left out, and a traced image through to the sheet.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, GradientPaint, Paint as Fill};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer, StrokeAlign, StrokeLayer};
use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{Affine, BezPath, Rect, Shape};
use vectorcraft_geom::{FillRule, PathData, shapes};
use vectorcraft_modifiers::Stencil;

use crate::{compute, gather};

fn fill(c: Color) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Fill::solid(c)))], ..Appearance::default() }
}

fn rect_node(id: u64, r: Rect, c: Color) -> Node {
    Node::path(NodeId(id), shapes::rectangle(r), fill(c))
}

fn group(id: u64, kids: Vec<Node>) -> Node {
    Node::group(NodeId(id), kids.into_iter().map(Arc::new).collect())
}

fn red() -> Color {
    Color::Rgb { r: 1.0, g: 0.0, b: 0.0 }
}

fn blue() -> Color {
    Color::Rgb { r: 0.0, g: 0.0, b: 1.0 }
}

fn area(p: &BezPath) -> f64 {
    crate::ops::simplify(p).unwrap().area().abs()
}

#[test]
fn a_group_gathers_its_members_bottom_first() {
    let g = group(1, vec![rect_node(2, Rect::new(0.0, 0.0, 100.0, 100.0), red()), rect_node(3, Rect::new(50.0, 50.0, 150.0, 150.0), blue())]);
    let out = gather(&g, &[], None);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.paints.len(), 2);
    assert_eq!(out.paints[0].colour, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(out.paints[1].colour, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!((out.paints[0].order, out.paints[1].order), (0, 1));
    assert_eq!((out.sources[0].node, out.sources[1].node), (NodeId(2), NodeId(3)));
    assert!((area(&out.paints[1].path) - 10_000.0).abs() < 1e-6);
}

#[test]
fn hidden_objects_guides_and_empty_paints_are_left_out_silently() {
    let mut hidden = rect_node(2, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    hidden.visible = false;
    let mut guide = rect_node(3, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    if let NodeKind::Path { guide: g, .. } = &mut guide.kind {
        *g = true;
    }
    let none = Node::path(NodeId(4), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
    let mut clear = rect_node(5, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    clear.opacity = 0.0;
    let out = gather(&group(1, vec![hidden, guide, none, clear]), &[], None);
    assert!(out.paints.is_empty() && out.warnings.is_empty(), "{out:?}");
}

#[test]
fn compound_paths_keep_their_holes_by_fill_rule() {
    let mut ring = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    // Same direction inside: a hole only under even-odd.
    ring.extend(shapes::rectangle(Rect::new(25.0, 25.0, 75.0, 75.0)).to_bezpath());
    let kids: Vec<Arc<Node>> = crate::ops::split_subpaths(&ring)
        .iter()
        .enumerate()
        .map(|(i, p)| Arc::new(Node::path(NodeId(10 + i as u64), PathData::from_bezpath(p), Appearance::default())))
        .collect();
    for (rule, want) in [(FillRule::EvenOdd, 7_500.0), (FillRule::NonZero, 10_000.0)] {
        let mut c = Node::new(NodeId(1), NodeKind::Compound { children: kids.clone(), rule });
        c.appearance = fill(red()).into();
        let out = gather(&c, &[], None);
        assert_eq!(out.paints.len(), 1);
        assert!((area(&out.paints[0].path) - want).abs() < 1e-6, "{rule:?}");
    }
}

#[test]
fn strokes_are_outlined_and_aligned() {
    let r = Rect::new(0.0, 0.0, 100.0, 100.0);
    let mut stroke = StrokeLayer::new(Fill::solid(blue()), 10.0);
    stroke.join = vectorcraft_doc::appearance::LineJoin::Miter;
    let cases = [
        (StrokeAlign::Center, 110.0 * 110.0 - 90.0 * 90.0),
        (StrokeAlign::Inside, 100.0 * 100.0 - 80.0 * 80.0),
        (StrokeAlign::Outside, 120.0 * 120.0 - 100.0 * 100.0),
    ];
    for (align, want) in cases {
        let s = StrokeLayer { align, ..stroke.clone() };
        let ap =
            Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Fill::solid(red()))), AppearanceItem::Stroke(s)], ..Appearance::default() };
        let n = Node::path(NodeId(1), shapes::rectangle(r), ap);
        let out = gather(&n, &[], None);
        assert_eq!(out.paints.len(), 2, "fill below the stroke");
        assert!(!out.sources[0].is_stroke && out.sources[1].is_stroke);
        assert!((area(&out.paints[1].path) - want).abs() < 1.0, "{align:?}: {}", area(&out.paints[1].path));
    }
}

#[test]
fn a_clip_group_clips_the_rest_to_its_first_member() {
    let clip = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default());
    let art = rect_node(3, Rect::new(-100.0, -100.0, 100.0, 100.0), red());
    let g = Node::new(NodeId(1), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(art)], clip: true });
    let out = gather(&g, &[], None);
    assert_eq!(out.paints.len(), 1);
    assert!((area(&out.paints[0].path) - 2_500.0).abs() < 1e-6);
}

#[test]
fn copies_place_the_design_again() {
    let n = rect_node(1, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    let out = gather(&n, &[Affine::IDENTITY, Affine::translate((100.0, 0.0))], None);
    assert_eq!(out.paints.len(), 2);
    assert_eq!(out.paints[1].path.bounding_box(), Rect::new(100.0, 0.0, 110.0, 10.0));
    assert_eq!(out.paints[1].order, 1);
}

#[test]
fn a_modifier_stack_inside_gathers_each_copy() {
    let mut inner = rect_node(2, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    vectorcraft_modifiers::set_node_source(
        &mut inner,
        Some(json!({"v": 1, "stack": [{"id": "m1", "kind": "linearArray", "count": 3, "offsetX": 20}]}).to_string()),
    );
    let out = gather(&group(1, vec![inner]), &[], None);
    assert_eq!(out.paints.len(), 3);
    let xs: Vec<f64> = out.paints.iter().map(|p| p.path.bounding_box().x0).collect();
    assert_eq!(xs, vec![0.0, 20.0, 40.0]);
}

#[test]
fn what_cant_be_cut_is_said_once() {
    let sym = Node::new(NodeId(2), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
    let sym2 = Node::new(NodeId(3), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
    let mut grad = rect_node(4, Rect::new(0.0, 0.0, 10.0, 10.0), red());
    // White to black.
    let g = GradientPaint::new(vectorcraft_color::Gradient::default());
    grad.appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Fill::Gradient(Box::new(g))))], ..Appearance::default() }.into();
    let out = gather(&group(1, vec![sym, sym2, grad]), &[], None);
    assert_eq!(out.paints.len(), 1);
    assert_eq!(out.paints[0].colour, [0.5, 0.5, 0.5, 1.0]);
    assert!(out.sources[0].gradient);
    assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
}

#[test]
fn hostile_nesting_stops_at_the_depth_cap() {
    let mut n = rect_node(0, Rect::new(0.0, 0.0, 1.0, 1.0), red());
    for i in 1..=(crate::MAX_GATHER_DEPTH as u64 + 5) {
        n = group(i, vec![n]);
    }
    let out = gather(&n, &[], None);
    assert!(out.paints.is_empty());
    assert_eq!(out.warnings.len(), 1);
    // Non-finite copies are skipped.
    let out = gather(&rect_node(1, Rect::new(0.0, 0.0, 1.0, 1.0), red()), &[Affine::scale(f64::NAN)], None);
    assert!(out.paints.is_empty());
}

/// The trace → stencil path: an image traced with VectorCraft's Image Trace, its result built as
/// the Image Trace command builds it (a group of filled paths, in the tracer's order), gathered
/// and cut. The red disc and the black square are cut (dark colours, `invert`), the white ground
/// stays paper.
#[test]
fn a_traced_image_cuts_its_dark_shapes() {
    let (w, h) = (200u32, 100u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let px = if (fx - 60.0).hypot(fy - 50.0) < 35.0 {
                [220, 20, 20, 255]
            } else if (140.0..180.0).contains(&fx) && (30.0..70.0).contains(&fy) {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            };
            rgba.extend(px);
        }
    }
    let raster = vectorcraft_trace::Raster::new(w, h, rgba).unwrap();
    let params = vectorcraft_trace::preset("3 colors").unwrap();
    let res = vectorcraft_trace::trace(&raster, &params);
    assert!(res.paths.len() >= 3, "{} paths", res.paths.len());
    let kids: Vec<Node> = res
        .paths
        .iter()
        .enumerate()
        .map(|(i, tp)| Node::path(NodeId(10 + i as u64), tp.path.clone(), fill(Color::rgb8(tp.color[0], tp.color[1], tp.color[2]))))
        .collect();
    let traced = group(1, kids);
    let design = gather(&traced, &[], None);
    assert!(design.warnings.is_empty(), "{:?}", design.warnings);
    assert_eq!(design.paints.len(), res.paths.len());
    let p = Stencil { invert: true, hoshi: false, ..Stencil::default() };
    let out = compute(&design.paints, &p, Rect::new(0.0, 0.0, f64::from(w), f64::from(h))).unwrap();
    assert!((out.mm_per_unit - 1.5).abs() < 1e-12);
    assert_eq!(out.stats.cut_pieces, 2, "{:?}", out.warnings);
    let want = (std::f64::consts::PI * 35.0 * 35.0 + 40.0 * 40.0) * 1.5 * 1.5;
    assert!((out.stats.area_cut_mm2 - want).abs() < want * 0.03, "{} vs {want}", out.stats.area_cut_mm2);
    assert_eq!(out.stats.islands, 0);
    assert!(out.colours.len() >= 3);
}
