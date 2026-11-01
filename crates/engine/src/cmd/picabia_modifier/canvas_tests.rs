//! PICABIA: modifier copies on the canvas: clicking, hovering or marqueeing a copy finds its
//! object (the front-most object wins), through the Selection tool and the hit test every tool
//! uses.

use serde_json::json;
use vectorcraft_doc::hit::{HitOptions, hit_test, marquee};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::tests::{arrayed, ex, rect};
use crate::{Session, ViewInfo};

fn click(s: &mut Session, x: f64, y: f64) {
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
}

#[test]
fn clicking_a_copy_selects_its_object() {
    // A 100×100 rectangle at 0,0 and four copies 120 apart.
    let (mut s, r) = arrayed();
    let leaf = |s: &Session, x: f64| hit_test(&s.doc().unwrap().doc, Point::new(x, 50.0), HitOptions::default()).map(|h| h.leaf);
    for x in [50.0, 170.0, 530.0] {
        assert_eq!(leaf(&s, x), Some(r), "x = {x}");
    }
    assert_eq!(leaf(&s, 110.0), None, "between the copies");
    s.select_tool("selection", ViewInfo::default()).unwrap();
    ex(&mut s, "select.none", json!({}));
    click(&mut s, 530.0, 50.0);
    assert_eq!(s.doc().unwrap().selection.objects, [r]);
    click(&mut s, 110.0, 50.0);
    assert!(s.doc().unwrap().selection.objects.is_empty());
    // An object above a copy still wins there; the copy wins over one below it.
    let top = rect(&mut s, 500.0);
    assert_eq!(leaf(&s, 530.0), Some(top));
    ex(&mut s, "object.arrange.sendToBack", json!({"ids": [top.0]}));
    assert_eq!(leaf(&s, 530.0), Some(r));
}

#[test]
fn a_copy_inside_a_group_hits_with_the_groups_ancestry() {
    let (mut s, r) = arrayed();
    let g = super::tests::id_of(&ex(&mut s, "object.group", json!({"ids": [r.0]}))["id"]);
    let h = hit_test(&s.doc().unwrap().doc, Point::new(290.0, 50.0), HitOptions::default()).unwrap();
    assert_eq!(h.leaf, r);
    assert!(h.ancestry.contains(&g) && h.ancestry.last() == Some(&r), "{:?}", h.ancestry);
    assert_eq!(h.ancestry.iter().filter(|a| **a == r).count(), 1, "the object once in the chain");
}

#[test]
fn a_marquee_over_a_copy_takes_its_object() {
    let (s, r) = arrayed();
    let d = &s.doc().unwrap().doc;
    assert_eq!(marquee(d, Rect::new(500.0, 0.0, 520.0, 30.0), None, false), [r]);
    assert!(marquee(d, Rect::new(102.0, 0.0, 118.0, 30.0), None, false).is_empty());
    // Through the Selection tool's drag too.
    let mut s = s;
    s.select_tool("selection", ViewInfo::default()).unwrap();
    ex(&mut s, "select.none", json!({}));
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, 505.0, -20.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 515.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 515.0, 10.0), v).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, [r]);
}

#[test]
fn direct_selections_marquee_takes_the_members_of_a_groups_copies() {
    let mut s = super::tests::session();
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 200.0));
    let g = super::tests::id_of(&ex(&mut s, "object.group", json!({"ids": [a.0, b.0]}))["id"]);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "linearArray", "props": {"count": 2, "offsetX": 0, "offsetY": 300}}));
    let d = &s.doc().unwrap().doc;
    // Across the left edge of the lower copy of `b` only (b is 200..300 x 0..100; the copy 300 lower).
    assert_eq!(marquee(d, Rect::new(190.0, 340.0, 210.0, 360.0), None, true), [b]);
    assert_eq!(marquee(d, Rect::new(-10.0, 290.0, 400.0, 420.0), None, true), [a, b], "each member once");
}

/// A click follows what is drawn: past the nested budget a member draws once in each of its
/// group's copies, so its own copies (not drawn) aren't clicked; and a click stays quick.
#[test]
fn clicks_follow_the_nested_budget() {
    let mut s = super::tests::session();
    let r = rect(&mut s, 0.0);
    // The member's own copies sit far below; the group's copies go right, 200 of them.
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"count": 100, "offsetX": 0, "offsetY": 1000}}));
    let g = super::tests::id_of(&ex(&mut s, "object.group", json!({"ids": [r.0]}))["id"]);
    ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "linearArray", "props": {"count": 200, "offsetX": 200}}));
    let d = &s.doc().unwrap().doc;
    let leaf = |x: f64, y: f64| hit_test(d, Point::new(x, y), HitOptions::default()).map(|h| h.leaf);
    // 200 × 100 = 20 000 over the 10 000 budget: the member draws once in each group copy.
    assert_eq!(leaf(250.0, 50.0), Some(r), "a group copy is clicked");
    assert_eq!(leaf(250.0, 1050.0), None, "the member's own copies aren't drawn, so they aren't clicked");
    // Under the budget they are.
    ex(&mut s, "modifier.set", json!({"ids": [g.0], "index": 0, "props": {"count": 50}}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(hit_test(d, Point::new(250.0, 1050.0), HitOptions::default()).map(|h| h.leaf), Some(r));
    // Every click and hover tries at most MAX_PLACEMENTS placements.
    let t = std::time::Instant::now();
    for _ in 0..20 {
        let _ = hit_test(d, Point::new(-500.0, -500.0), HitOptions::default());
    }
    assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
}

/// Everything drawn within the budgets stays clickable: a group copied 10 times whose member is
/// copied 1 000 times (an array's most; 10 000 nested, all drawn: 10 010 placements, past the old
/// cap of 10 000), and 100 × 100.
#[test]
fn every_drawn_nested_copy_is_clickable() {
    for (outer, inner) in [(10u32, 1000u32), (100, 100)] {
        let mut s = super::tests::session();
        let a = super::tests::id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"]);
        ex(&mut s, "modifier.add", json!({"ids": [a.0], "kind": "linearArray", "props": {"count": inner, "offsetX": 0, "offsetY": 30}}));
        let g = super::tests::id_of(&ex(&mut s, "object.group", json!({"ids": [a.0]}))["id"]);
        ex(&mut s, "modifier.add", json!({"ids": [g.0], "kind": "linearArray", "props": {"count": outer, "offsetX": 200}}));
        let d = &s.doc().unwrap().doc;
        let leaf = |x: f64, y: f64| hit_test(d, Point::new(x, y), HitOptions::default()).map(|h| h.leaf);
        let last = 30.0 * f64::from(inner - 1) + 10.0;
        assert_eq!(leaf(10.0, 10.0), Some(a), "{outer} x {inner}: the original");
        assert_eq!(leaf(10.0, last), Some(a), "{outer} x {inner}: the original group's last member copy");
        assert_eq!(leaf(200.0 * f64::from(outer - 1) + 10.0, last), Some(a), "{outer} x {inner}: the last copy's last member copy");
    }
}
