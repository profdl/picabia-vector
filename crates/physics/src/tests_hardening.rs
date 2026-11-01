//! Degenerate and oversized input (PIC30.2a): geometry that collapses in f32, outlines past the
//! caps, non-simple rings, the per-run piece budget, and a turned body's pose. These call the
//! crate directly (no panic catching): a panic here would abort the web build.

use std::sync::Arc;

use rapier2d::prelude::Vector;
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, shapes};

use crate::settings::{BodyType, ColliderType, Settings};
use crate::shape::{MAX_POINTS_PER_BODY, body_shape, clean_ring, convex_parts, is_simple};
use crate::sim::{BodyInput, SimParams, convex, simulate};

const BOARD: Rect = Rect::new(0.0, 0.0, 400.0, 300.0);
const FIDELITIES: [ColliderType; 4] = [ColliderType::Auto, ColliderType::Precise, ColliderType::Fast, ColliderType::Box];

fn node(id: u64, path: PathData) -> Node {
    Node::path(NodeId(id), path, Appearance::default_art())
}

fn ring(pts: &[(f64, f64)], closed: bool) -> PathData {
    PathData::single(SubPath::new(pts.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), closed))
}

/// Simulate `n` with every fidelity, dynamic and static, a few steps.
fn every_way(n: &Node) {
    let shape = body_shape(n, None).unwrap();
    let mut bodies = vec![];
    for c in FIDELITIES {
        for t in [BodyType::Dynamic, BodyType::Static] {
            bodies.push(BodyInput { settings: Settings { collider: c, body_type: t, ..Settings::default() }, shape: shape.clone() });
        }
    }
    let out = simulate(&bodies, &SimParams { walls: Some(BOARD), max_steps: 30, ..SimParams::default() });
    assert_eq!(out.outcomes.len(), bodies.len());
}

#[test]
fn coincident_f32_points_never_reach_parry_hull() {
    assert!(convex(&[Vector::new(1.0, 1.0); 5]).is_none());
    assert!(convex(&[Vector::new(0.0, 0.0), Vector::new(1.0, 0.0), Vector::new(2.0, 0.0)]).is_none(), "collinear");
    assert!(convex(&[Vector::new(0.0, 0.0), Vector::new(1e-9, 0.0), Vector::new(0.0, 1e-9)]).is_none(), "sub-micro");
    assert!(convex(&[Vector::new(f32::NAN, 0.0), Vector::new(1.0, 0.0), Vector::new(0.0, 1.0)]).is_none());
    assert!(convex(&[Vector::new(0.0, 0.0), Vector::new(1.0, 0.0), Vector::new(0.0, 1.0)]).is_some());
}

#[test]
fn a_tiny_triangle_beside_a_big_rectangle_collapses_in_f32_and_falls_back() {
    // In the group's body space (metres from its centre, ~1.5 m away) the triangle's points are
    // 1e-8 m apart: one f32 point.
    let big = Arc::new(node(1, shapes::rectangle(Rect::new(10.0, 10.0, 310.0, 40.0))));
    let tiny = Arc::new(node(2, ring(&[(10.0, 10.0), (10.000001, 10.0), (10.0, 10.000001)], true)));
    let g = Node::group(NodeId(3), vec![big, tiny.clone()]);
    every_way(&g);
    // The tiny triangle alone is refused (no size), never handed to rapier.
    assert!(body_shape(&tiny, None).is_err());
    // A sub-micro triangle 0.02 pt wide: big enough to be read, collapsing near the origin.
    let small = node(4, ring(&[(200.0, 100.0), (200.02, 100.0), (200.0, 100.000_000_1)], true));
    every_way(&small);
    // A precise ring with many f32-coincident points around a real outline.
    let mut pts: Vec<(f64, f64)> = (0..40).map(|i| (100.0 + 1e-9 * f64::from(i), 50.0)).collect();
    pts.extend([(160.0, 50.0), (160.0, 90.0), (100.0, 90.0)]);
    every_way(&node(5, ring(&pts, true)));
}

#[test]
fn a_hundred_thousand_subpaths_stay_within_the_caps() {
    let subs: Vec<SubPath> = (0..100_000)
        .map(|i| {
            let (x, y) = (f64::from(i % 300), f64::from(i / 300) * 0.5);
            SubPath::new([(x, y), (x + 0.5, y), (x, y + 0.5)].iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), true)
        })
        .collect();
    let n = node(1, PathData::new(subs));
    let shape = body_shape(&n, None).unwrap();
    assert!(shape.pieces.len() <= crate::shape::MAX_PIECES_PER_BODY, "{}", shape.pieces.len());
    assert!(shape.all_points().len() <= MAX_POINTS_PER_BODY, "{}", shape.all_points().len());
    assert!(shape.notes.iter().any(|m| m.contains("left out")), "{:?}", shape.notes);
}

#[test]
fn a_huge_ring_is_thinned_evenly_and_keeps_its_shape() {
    let n = 300_000u32;
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = f64::from(i) / f64::from(n) * std::f64::consts::TAU;
            (200.0 + 100.0 * a.cos(), 150.0 + 100.0 * a.sin())
        })
        .collect();
    let shape = body_shape(&node(1, ring(&pts, true)), None).unwrap();
    let all = shape.all_points();
    assert!(all.len() <= MAX_POINTS_PER_BODY);
    // Spread all round: the outline's area is the circle's, not a chord's.
    let area = crate::shape::signed_area2(&all).abs() / 2.0;
    let circle = std::f64::consts::PI * 100.0 * 100.0;
    assert!((area - circle).abs() / circle < 0.01, "{area} vs {circle}");
    assert!(shape.notes.iter().any(|m| m.contains("kept")), "{:?}", shape.notes);
}

#[test]
fn ring_floors_never_push_the_total_over_the_cap() {
    // 500 rings of 40 points each.
    let subs: Vec<SubPath> = (0..500u32)
        .map(|k| {
            let (cx, cy) = (f64::from(k % 25) * 15.0, f64::from(k / 25) * 15.0);
            SubPath::new(
                (0..40u32)
                    .map(|i| {
                        let a = f64::from(i) / 40.0 * std::f64::consts::TAU;
                        Anchor::corner(Point::new(cx + 5.0 * a.cos(), cy + 5.0 * a.sin()))
                    })
                    .collect(),
                true,
            )
        })
        .collect();
    let shape = body_shape(&node(1, PathData::new(subs)), None).unwrap();
    assert!(shape.all_points().len() <= MAX_POINTS_PER_BODY, "{}", shape.all_points().len());
}

#[test]
fn non_simple_rings_are_not_ear_clipped() {
    // An asymmetric bow tie: its edges cross.
    let bow = [(0.0, 0.0), (100.0, 60.0), (100.0, 0.0), (0.0, 20.0)].map(|(x, y)| Point::new(x, y));
    assert!(!is_simple(&bow));
    assert!(convex_parts(&bow).is_none());
    // Repeated and collinear points are cleaned first; the square still splits.
    let square =
        [(0.0, 0.0), (0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)].map(|(x, y)| Point::new(x, y));
    assert_eq!(clean_ring(&square).len(), 4);
    let parts = convex_parts(&square).unwrap();
    let area: f64 = parts.iter().map(|p| crate::shape::signed_area2(p).abs() / 2.0).sum();
    assert!((area - 10_000.0).abs() < 1e-6);
    // All collinear: nothing to clip.
    let line = [(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)].map(|(x, y)| Point::new(x, y));
    assert!(convex_parts(&line).is_none());
    // A ring touching itself at a point isn't simple either.
    let pinch = [(0.0, 0.0), (50.0, 50.0), (100.0, 0.0), (100.0, 100.0), (50.0, 50.0), (0.0, 100.0)].map(|(x, y)| Point::new(x, y));
    assert!(!is_simple(&pinch));
    every_way(&node(1, ring(&[(100.0, 50.0), (200.0, 110.0), (200.0, 50.0), (100.0, 70.0)], true)));
}

#[test]
fn the_run_piece_budget_boxes_bodies_past_it() {
    // A zigzag gear: about a hundred convex parts when split.
    let gear: Vec<(f64, f64)> = (0..200u32)
        .map(|i| {
            let a = f64::from(i) / 200.0 * std::f64::consts::TAU;
            let r = if i % 2 == 0 { 10.0 } else { 6.0 };
            (r * a.cos(), r * a.sin())
        })
        .collect();
    let mut bodies = vec![];
    for k in 0..400u32 {
        let (x, y) = (20.0 + f64::from(k % 20) * 18.0, 20.0 + f64::from(k / 20) * 13.0);
        let pts: Vec<(f64, f64)> = gear.iter().map(|(gx, gy)| (gx + x, gy + y)).collect();
        let shape = body_shape(&node(k.into(), ring(&pts, true)), None).unwrap();
        bodies.push(BodyInput { settings: Settings { collider: ColliderType::Precise, ..Settings::default() }, shape });
    }
    let out = simulate(&bodies, &SimParams { walls: Some(BOARD), max_steps: 1, ..SimParams::default() });
    assert!(out.notes.iter().any(|m| m.contains("collider pieces")), "{:?}", out.notes);
}

#[test]
fn a_box_sliding_onto_a_slope_rests_turned_with_its_edge_on_the_slope() {
    // A static slope from (0, 200) to (400, 280): about 11°.
    let slope = node(1, ring(&[(0.0, 200.0), (400.0, 280.0), (400.0, 300.0), (0.0, 300.0)], true));
    let r = Rect::new(150.0, 100.0, 200.0, 130.0);
    let b = node(2, shapes::rectangle(r));
    let still = Settings { body_type: BodyType::Static, ..Settings::default() };
    let grippy = Settings { friction: 1.0, restitution: 0.0, ..Settings::default() };
    let bodies = [
        BodyInput { settings: still, shape: body_shape(&slope, None).unwrap() },
        BodyInput { settings: grippy, shape: body_shape(&b, None).unwrap() },
    ];
    let out = simulate(&bodies, &SimParams { walls: Some(BOARD), ..SimParams::default() });
    let a: Affine = out.outcomes[1].moved.unwrap();
    // Its turn is the slope's.
    let c = a.as_coeffs();
    let turned = c[1].atan2(c[0]).to_degrees();
    let slope_deg = (80.0f64 / 400.0).atan().to_degrees();
    assert!((turned - slope_deg).abs() < 1.5, "turned {turned}°, slope {slope_deg}°");
    // Its two lower corners, moved by the pose, lie on the slope's top edge.
    let on_slope = |p: Point| 200.0 + p.x * 80.0 / 400.0;
    for corner in [Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)] {
        let p = a * corner;
        assert!((p.y - on_slope(p)).abs() < 1.5, "{p:?} vs slope y {}", on_slope(p));
    }
}
