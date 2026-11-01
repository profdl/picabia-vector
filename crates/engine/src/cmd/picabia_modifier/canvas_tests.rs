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
