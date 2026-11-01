//! PICABIA: modifier copies in the renderer: drawing, bounds, culling, redraws after an edit.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::*;
use crate::{Changed, RenderOptions, Renderer, changed, painted_bounds, view_pixels};

fn linear(count: u32, dx: f64) -> String {
    format!(r#"{{"v":1,"stack":[{{"id":"m1","enabled":true,"kind":"linearArray","count":{count},"offsetX":{dx},"offsetY":0}}]}}"#)
}

fn set_stack(d: &mut Document, id: NodeId, src: Option<&str>) {
    vectorcraft_modifiers::set_node_source(d.node_mut(id).unwrap(), src.map(str::to_string));
}

fn red_rect(d: &mut Document, r: Rect) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0))
}

/// A red 20×20 rectangle at (10, 10) on the first layer.
fn doc() -> (Document, NodeId) {
    let mut d = Document::new(400.0, 100.0);
    let l = d.layers[0].id;
    let n = red_rect(&mut d, Rect::new(10.0, 10.0, 30.0, 30.0));
    let r = d.insert(Some(l), 0, n).unwrap();
    (d, r)
}

/// `region` of `d` at 1 px a point, on white.
fn render(d: &Document, region: Rect) -> crate::Rendered {
    Renderer::new().render_region(d, region, 1.0, true)
}

fn px(p: &crate::Rendered, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width + x) * 4) as usize;
    [p.pixels[i], p.pixels[i + 1], p.pixels[i + 2], p.pixels[i + 3]]
}

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn a_three_copy_linear_array_draws_three_rectangles() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 40.0)));
    let pix = render(&d, Rect::new(0.0, 0.0, 140.0, 40.0));
    for x in [20, 60, 100] {
        assert_eq!(px(&pix, x, 20), RED, "copy at x = {x}");
    }
    for x in [40, 80, 120] {
        assert_eq!(px(&pix, x, 20), WHITE, "gap at x = {x}");
    }
}

#[test]
fn objects_without_stacks_draw_and_measure_as_before() {
    let (mut d, r) = doc();
    let region = Rect::new(0.0, 0.0, 140.0, 40.0);
    let before = render(&d, region).pixels;
    let pb = painted_bounds(&d, d.node(r).unwrap()).unwrap();
    // A damaged stack, an empty one, or one with every row off draws the object once.
    let off = r#"{"v":1,"stack":[{"id":"m1","enabled":false,"kind":"linearArray","count":3,"offsetX":40}]}"#;
    for src in ["not json", r#"{"v":1,"stack":[]}"#, off, r#"{"v":1,"stack":[{"id":"m1","kind":"linearArray","count":1}]}"#] {
        set_stack(&mut d, r, Some(src));
        assert!(render(&d, region).pixels == before, "{src}");
        assert_eq!(painted_bounds(&d, d.node(r).unwrap()), Some(pb), "{src}");
        assert!(art(&d, d.node(r).unwrap()).is_none(), "{src}");
    }
}

#[test]
fn bounds_cover_every_copy_and_copies_of_an_off_view_original_are_drawn() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 200.0)));
    let pb = painted_bounds(&d, d.node(r).unwrap()).unwrap();
    assert!(pb.x0 <= 10.0 && pb.x1 >= 430.0, "{pb:?}");
    // Only the third copy (410..430) is in view.
    let pix = render(&d, Rect::new(400.0, 0.0, 440.0, 40.0));
    assert_eq!(px(&pix, 20, 20), RED);
}

#[test]
fn a_group_measures_and_draws_its_members_copies() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 40.0)));
    let l = d.layers[0].id;
    let gid = d.alloc_id();
    let g = d.wrap(&[r], |c| Node::group(gid, c)).unwrap();
    assert_eq!(d.parent_of(r), Some(g));
    assert!(painted_bounds(&d, d.node(g).unwrap()).unwrap().x1 >= 110.0);
    let pix = render(&d, Rect::new(50.0, 0.0, 130.0, 40.0));
    assert_eq!(px(&pix, 50, 20), RED, "the third copy (90..110) shows at view x 40..60");
    // A stacked group draws its members in every copy.
    set_stack(&mut d, r, None);
    set_stack(&mut d, g, Some(&linear(3, 40.0)));
    let pix = render(&d, Rect::new(0.0, 0.0, 140.0, 40.0));
    assert_eq!(px(&pix, 100, 20), RED);
    assert!(painted_bounds(&d, d.node(g).unwrap()).unwrap().x1 >= 110.0);
    assert_eq!(d.parent_of(g), Some(l));
}

#[test]
fn transparency_composites_the_copies_once_and_effects_draw_on_each() {
    let (mut d, r) = doc();
    // Two copies overlapping by half at 50% opacity: the overlap is no darker than the rest.
    set_stack(&mut d, r, Some(&linear(2, 10.0)));
    d.node_mut(r).unwrap().opacity = 0.5;
    let pix = render(&d, Rect::new(0.0, 0.0, 60.0, 40.0));
    assert_eq!(px(&pix, 15, 20), px(&pix, 25, 20), "one group: the overlap isn't drawn twice");
    assert_eq!(px(&pix, 35, 20), px(&pix, 15, 20));
    // A drop shadow is drawn under each copy.
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(2, 100.0)));
    d.node_mut(r).unwrap().appearance.effects.push(vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 8.0, "y": 8.0, "blur": 0.0, "opacity": 100.0}),
        visible: true,
    });
    let pix = render(&d, Rect::new(0.0, 0.0, 160.0, 60.0));
    for x in [35, 135] {
        assert_ne!(px(&pix, x, 35), WHITE, "a shadow beside the copy at x = {}", x - 25);
    }
}

#[test]
fn outline_mode_draws_every_copy() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 40.0)));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), outline: true, ..Default::default() };
    let pix = Renderer::new().render_region_with(&d, Rect::new(0.0, 0.0, 140.0, 40.0), 1.0, &opts);
    // The left edge of the third copy (x 90) is drawn; its inside isn't filled.
    assert_ne!(px(&pix, 90, 20), WHITE);
    assert_eq!(px(&pix, 100, 20), WHITE);
}

/// Draws `new` again only where [`changed`] says it changed from `old`, over `old` drawn whole,
/// and checks that this is `new` drawn whole.
fn redraw_matches(old: &Document, new: &Document, what: &str) {
    const W: u32 = 560;
    const H: u32 = 150;
    let view = Affine::translate((3.3, 2.7)) * Affine::scale(1.37);
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let before = r.render(old, W, H, view, &opts);
    let after = r.render(new, W, H, view, &opts);
    let fresh = Renderer::new().render(new, W, H, view, &opts);
    assert!(after.pixels == fresh.pixels, "{what}: a renderer that drew `old` draws `new` like a fresh one");
    let mut patched = before.pixels.clone();
    match changed(old, new) {
        Changed::Everything => patched = after.pixels.clone(),
        Changed::Nothing => {}
        Changed::Area(area) => {
            if let Some([x0, y0, x1, y1]) = view_pixels(area, view, W, H) {
                let part = Renderer::new().render(new, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view, &opts);
                for y in 0..y1 - y0 {
                    let (from, to) = ((y * (x1 - x0) * 4) as usize, (((y0 + y) * W + x0) * 4) as usize);
                    let n = ((x1 - x0) * 4) as usize;
                    patched[to..to + n].copy_from_slice(&part.pixels[from..from + n]);
                }
            }
        }
    }
    let at = |p: &[u8], i: u32| p[(i * 4) as usize..(i * 4 + 4) as usize].to_vec();
    // Within 1 of 255 a channel: a moved object's anti-aliased edge drawn from the dirty rect's
    // origin can differ by one step from the whole view's, upstream too: a plain object moved by
    // the same amount (`redraws_after_an_edit_match_full_renders_without_stacks`) differs at the
    // same pixel, by 1. The dirty rect itself covers every changed pixel.
    let near = |a: Vec<u8>, b: Vec<u8>| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 1);
    let diff: Vec<_> = (0..W * H)
        .filter(|i| !near(at(&patched, *i), at(&after.pixels, *i)))
        .map(|i| (i % W, i / W, at(&patched, i), at(&after.pixels, i)))
        .take(5)
        .collect();
    assert!(diff.is_empty(), "{what}: drawn again only where it changed ({:?}), it differs from drawing it all at {diff:?}", changed(old, new));
}

#[test]
fn redraws_after_an_edit_match_full_renders_when_stacks_change() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 40.0)));
    type Step = Box<dyn Fn(&mut Document)>;
    let steps: [(&str, Step); 4] = [
        ("more copies", Box::new(move |d| set_stack(d, r, Some(&linear(5, 50.0))))),
        ("fewer copies", Box::new(move |d| set_stack(d, r, Some(&linear(2, 50.0))))),
        ("moved", Box::new(move |d| d.node_mut(r).unwrap().transform(Affine::translate((30.0, 20.0)), false))),
        ("stack removed", Box::new(move |d| set_stack(d, r, None))),
    ];
    for (what, step) in steps {
        let mut nd = d.clone();
        step(&mut nd);
        redraw_matches(&d, &nd, what);
        d = nd;
    }
}

#[test]
fn moving_a_copy_to_shape_target_redraws_its_sources_copies() {
    let (mut d, r) = doc();
    let l = d.layers[0].id;
    let ring = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(100.0, 10.0, 200.0, 90.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    let t = d.insert(Some(l), 1, ring).unwrap();
    let src = format!(r#"{{"v":1,"stack":[{{"id":"m1","kind":"copyToShape","target":"{}","mode":"alongPath","count":6}}]}}"#, t.0);
    set_stack(&mut d, r, Some(&src));
    assert_eq!(art(&d, d.node(r).unwrap()).unwrap().children().unwrap().len(), 6);
    let mut nd = d.clone();
    nd.node_mut(t).unwrap().transform(Affine::translate((120.0, 0.0)), false);
    // The source itself didn't change: only the dependents seam says its copies moved.
    assert!(std::ptr::eq(d.node(r).unwrap(), nd.node(r).unwrap()) || d.node(r) == nd.node(r));
    redraw_matches(&d, &nd, "target moved");
    // A renderer that drew before the move draws the copies where the target is now.
    let mut renderer = Renderer::new();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let _ = renderer.render(&d, 400, 100, Affine::IDENTITY, &opts);
    let after = renderer.render(&nd, 400, 100, Affine::IDENTITY, &opts);
    let fresh = Renderer::new().render(&nd, 400, 100, Affine::IDENTITY, &opts);
    assert!(after.pixels == fresh.pixels, "no stale copies kept by the source's identity");
}

#[test]
fn nested_stacks_share_one_copy_budget() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(1000, 0.05)));
    let gid = d.alloc_id();
    let g = d.wrap(&[r], |c| Node::group(gid, c)).unwrap();
    set_stack(&mut d, g, Some(&linear(1000, 0.05)));
    let a = art(&d, d.node(g).unwrap()).unwrap();
    assert_eq!(a.children().unwrap().len(), 1000);
    let inner = a.children().unwrap()[0].children().unwrap()[0].clone();
    assert!(!vectorcraft_modifiers::has_modifiers(&inner), "the members draw once in each copy past the budget");
    // Under the budget the members keep their copies.
    set_stack(&mut d, g, Some(&linear(3, 40.0)));
    set_stack(&mut d, r, Some(&linear(3, 0.5)));
    let a = art(&d, d.node(g).unwrap()).unwrap();
    assert!(vectorcraft_modifiers::has_modifiers(&a.children().unwrap()[2].children().unwrap()[0]));
    assert!(matches!(a.kind, NodeKind::Group { .. }));
    assert!(!vectorcraft_modifiers::has_modifiers(&a), "the art carries no stack");
    let _ = Arc::strong_count(&a);
}

/// The same move on a plain object (no stack, upstream's own drawing): its redraw differs from
/// the whole view by one step at an edge pixel too, which is why `redraw_matches` allows 1.
#[test]
fn redraws_after_an_edit_match_full_renders_without_stacks() {
    let (d, r) = doc();
    let mut nd = d.clone();
    nd.node_mut(r).unwrap().transform(Affine::translate((30.0, 20.0)), false);
    redraw_matches(&d, &nd, "plain object moved");
    // Exactly: the patched redraw and the whole view differ, by at most 1.
    let view = Affine::translate((3.3, 2.7)) * Affine::scale(1.37);
    let opts = RenderOptions::default();
    let whole = Renderer::new().render(&nd, 560, 150, view, &opts);
    let Changed::Area(area) = changed(&d, &nd) else { panic!("an area changed") };
    let [x0, y0, x1, y1] = view_pixels(area, view, 560, 150).unwrap();
    let part = Renderer::new().render(&nd, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view, &opts);
    let (mut same, mut off_by_one) = (true, true);
    for y in 0..y1 - y0 {
        for x in 0..x1 - x0 {
            let a = ((y * (x1 - x0) + x) * 4) as usize;
            let b = (((y0 + y) * 560 + x0 + x) * 4) as usize;
            for k in 0..4 {
                let (p, q) = (part.pixels[a + k], whole.pixels[b + k]);
                same &= p == q;
                off_by_one &= p.abs_diff(q) <= 1;
            }
        }
    }
    assert!(!same && off_by_one, "upstream's own partial redraw differs by one step somewhere, never more");
}

fn effect(id: &str, params: serde_json::Value) -> vectorcraft_doc::Effect {
    vectorcraft_doc::Effect { id: id.into(), params, visible: true }
}

#[test]
fn a_stack_that_makes_no_copies_draws_and_measures_exactly_as_upstream() {
    let (mut d, r) = doc();
    {
        let n = d.node_mut(r).unwrap();
        n.appearance.effects.push(effect("distort.zigZag", serde_json::json!({"size": 4.0, "ridges": 5.0})));
        n.appearance.effects.push(effect("stylize.dropShadow", serde_json::json!({"x": 6.0, "y": 6.0, "blur": 2.0, "opacity": 75.0})));
    }
    let region = Rect::new(0.0, 0.0, 80.0, 60.0);
    let want = render(&d, region).pixels;
    let want_bounds = painted_bounds(&d, d.node(r).unwrap());
    // One unmoved copy, and a copy-to-shape row whose target is gone.
    let gone = r#"{"v":1,"stack":[{"id":"m1","kind":"copyToShape","target":"99999"}]}"#;
    for src in [r#"{"v":1,"stack":[{"id":"m1","kind":"linearArray","count":1}]}"#, gone] {
        let mut s = d.clone();
        set_stack(&mut s, r, Some(src));
        assert!(has_stack(s.node(r).unwrap()) && !draws_copies(&s, s.node(r).unwrap()), "{src}");
        assert!(render(&s, region).pixels == want, "{src}: drawn as upstream draws it");
        assert_eq!(painted_bounds(&s, s.node(r).unwrap()), want_bounds, "{src}");
    }
}

#[test]
fn a_copy_to_shape_source_inside_a_group_follows_its_moved_target() {
    let (mut d, r) = doc();
    let l = d.layers[0].id;
    let ring = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(100.0, 10.0, 200.0, 90.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    let t = d.insert(Some(l), 1, ring).unwrap();
    let src = format!(r#"{{"v":1,"stack":[{{"id":"m1","kind":"copyToShape","target":"{}","mode":"alongPath","count":6}}]}}"#, t.0);
    set_stack(&mut d, r, Some(&src));
    let gid = d.alloc_id();
    d.wrap(&[r], |c| Node::group(gid, c)).unwrap();
    let mut nd = d.clone();
    nd.node_mut(t).unwrap().transform(Affine::translate((150.0, 0.0)), false);
    redraw_matches(&d, &nd, "target of a grouped source moved");
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut renderer = Renderer::new();
    let _ = renderer.render(&d, 400, 100, Affine::IDENTITY, &opts);
    let after = renderer.render(&nd, 400, 100, Affine::IDENTITY, &opts);
    let fresh = Renderer::new().render(&nd, 400, 100, Affine::IDENTITY, &opts);
    assert!(after.pixels == fresh.pixels, "the group's kept bounds and art don't hold stale copies");
}

#[test]
fn copies_of_a_knockout_group_dont_knock_each_other_out() {
    let (mut d, r) = doc();
    d.node_mut(r).unwrap().opacity = 0.5;
    let gid = d.alloc_id();
    let g = d.wrap(&[r], |c| Node::group(gid, c)).unwrap();
    d.node_mut(g).unwrap().knockout = vectorcraft_doc::Knockout::On;
    // Two copies overlapping by half: in the overlap both show (darker than one alone).
    set_stack(&mut d, g, Some(&linear(2, 10.0)));
    let pix = render(&d, Rect::new(0.0, 0.0, 60.0, 40.0));
    let (one, both) = (px(&pix, 15, 20), px(&pix, 25, 20));
    assert!(both[1] < one[1], "overlap {both:?} darker than one copy {one:?}");
}

#[test]
fn nested_copies_multiply_and_count_copy_to_shape_members() {
    let (mut d, r) = doc();
    let l = d.layers[0].id;
    let ring = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(100.0, 10.0, 200.0, 90.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    let t = d.insert(Some(l), 1, ring).unwrap();
    // The member copies 100 times onto the ring; the group copies 90 times: 9 000 drawn objects
    // inside one budget of 10 000 (an inner count of 1, as without the document, would undercount).
    let src = format!(r#"{{"v":1,"stack":[{{"id":"m1","kind":"copyToShape","target":"{}","mode":"alongPath","count":100}}]}}"#, t.0);
    set_stack(&mut d, r, Some(&src));
    let gid = d.alloc_id();
    let g = d.wrap(&[r], |c| Node::group(gid, c)).unwrap();
    set_stack(&mut d, g, Some(&linear(90, 0.5)));
    let a = art(&d, d.node(g).unwrap()).unwrap();
    assert!(vectorcraft_modifiers::has_modifiers(&a.children().unwrap()[0].children().unwrap()[0]), "9 000: within the budget");
    set_stack(&mut d, g, Some(&linear(200, 0.5)));
    let a = art(&d, d.node(g).unwrap()).unwrap();
    assert!(!vectorcraft_modifiers::has_modifiers(&a.children().unwrap()[0].children().unwrap()[0]), "20 000: members draw once");
}

#[test]
fn an_art_over_the_node_cap_draws_once_with_a_reason() {
    let (mut d, r) = doc();
    let l = d.layers[0].id;
    // A group of 200 rectangles, copied 1 000 times: 201 000 nodes to clone.
    let kids: Vec<Arc<Node>> = (0..200).map(|i| Arc::new(red_rect(&mut d, Rect::new(f64::from(i), 0.0, f64::from(i) + 1.0, 1.0)))).collect();
    let gid = d.alloc_id();
    let g = d.insert(Some(l), 1, Node::group(gid, kids)).unwrap();
    set_stack(&mut d, g, Some(&linear(1000, 1.0)));
    let n = d.node(g).unwrap();
    assert!(art(&d, n).is_none() && !draws_copies(&d, n));
    assert!(too_big(&d, n).is_some_and(|m| m.contains("100000")), "{:?}", too_big(&d, n));
    assert!(too_big(&d, d.node(r).unwrap()).is_none());
}

#[test]
fn nothing_is_walked_for_a_document_without_stacks() {
    let (d, r) = doc();
    assert_eq!(doc_flags(&d), Flags::default());
    let mut nd = d.clone();
    nd.node_mut(r).unwrap().transform(Affine::translate((5.0, 0.0)), false);
    assert!(dependents(&d, &nd).is_none());
    let layer = d.layers[0].clone();
    assert!(!holds_copies(&layer));
    // A container is flagged once and kept by its identity.
    assert_eq!(subtree_flags(&layer), subtree_flags(&layer));
}

#[test]
fn the_art_is_kept_for_the_same_object() {
    let (mut d, r) = doc();
    set_stack(&mut d, r, Some(&linear(3, 40.0)));
    let n = d.node(r).unwrap();
    let (a, b) = (art(&d, n).unwrap(), art(&d, n).unwrap());
    assert!(Arc::ptr_eq(&a, &b));
}
