//! PIC31.3: poses carried onto the art: the rest pose leaves the art where it is, a cloth strip
//! bends the art along its centre line, jelly cells move and stretch it, live shapes become paths.

use std::sync::Arc;

use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, PathData, Point, Rect, shapes};

use crate::mesh::{MeshKind, SoftMesh, soft_mesh};
use crate::pose::pose_node;
use crate::shape::body_shape;
use crate::soft::{Preset, Soft};

fn node(id: u64, path: PathData) -> Node {
    Node::path(NodeId(id), path, Appearance::default_art())
}

fn mesh(n: &Node, p: Preset, res: f64) -> SoftMesh {
    soft_mesh(&body_shape(n, None).unwrap(), &Soft { resolution: Some(res), ..Soft::of(p) }, 10_000).unwrap()
}

fn anchors(n: &Node) -> Vec<Point> {
    let NodeKind::Path { path, .. } = &n.kind else { panic!("a path") };
    path.subpaths.iter().flat_map(|s| s.anchors.iter().map(|a| a.p)).collect()
}

fn bounds(n: &Node) -> Rect {
    n.geometric_bounds().unwrap()
}

fn near(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}

#[test]
fn the_rest_pose_leaves_the_art_in_place() {
    for (p, res) in [(Preset::Cloth, 10.0), (Preset::Jelly, 10.0)] {
        let n = node(1, shapes::rectangle(Rect::new(20.0, 30.0, 220.0, 50.0)));
        let m = mesh(&n, p.clone(), res);
        let out = pose_node(&n, &m, &m.rest).unwrap();
        assert!(near(bounds(&out), bounds(&n), 1e-6), "{p:?}: {:?}", bounds(&out));
        // The original corners are still anchors (pieces are added between them).
        for c in anchors(&n) {
            assert!(anchors(&out).iter().any(|q| (*q - c).hypot() < 1e-6), "{p:?}: corner {c:?}");
        }
    }
}

#[test]
fn a_cloth_strip_turned_upright_turns_the_art_with_it() {
    // A 200 × 10 strip; its centre line posed straight down from its left end.
    let n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 10.0)));
    let m = mesh(&n, Preset::Cloth, 10.0);
    let MeshKind::Strip(s) = &m.kind else { panic!("a strip") };
    let start = s.origin;
    let to: Vec<Point> = (0..m.rest.len()).map(|i| Point::new(start.x, start.y + i as f64 * 10.0)).collect();
    let out = pose_node(&n, &m, &to).unwrap();
    let b = bounds(&out);
    // Upright: 10 wide (the thickness across the line), 200 tall, around the line x = 0.
    assert!(near(b, Rect::new(-5.0, 5.0, 5.0, 205.0), 0.01), "{b:?}");
}

#[test]
fn a_cloth_strip_draped_in_a_curve_bends_the_art_smoothly() {
    let n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 6.0)));
    let m = mesh(&n, Preset::Cloth, 10.0);
    // Draped over a point: a half circle of radius 300/π.
    let r = 300.0 / std::f64::consts::PI;
    let count = m.rest.len();
    let to: Vec<Point> = (0..count)
        .map(|i| {
            let a = std::f64::consts::PI * (1.0 - i as f64 / (count - 1) as f64);
            Point::new(150.0 + r * a.cos(), 100.0 - r * a.sin())
        })
        .collect();
    let out = pose_node(&n, &m, &to).unwrap();
    // Every anchor of the bent art lies within the strip's half thickness of the arc.
    for p in anchors(&out) {
        let d = (p - Point::new(150.0, 100.0)).hypot();
        assert!((d - r).abs() < 3.0 + 0.2, "{p:?} is {d} from the centre (radius {r})");
    }
}

#[test]
fn jelly_moves_and_stretches_the_art_with_its_cells() {
    let n = node(1, shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 60.0)));
    let m = mesh(&n, Preset::Jelly, 10.0);
    let moved: Vec<Point> = m.rest.iter().map(|p| *p + vectorcraft_geom::Vec2::new(40.0, 25.0)).collect();
    let out = pose_node(&n, &m, &moved).unwrap();
    assert!(near(bounds(&out), Rect::new(40.0, 25.0, 140.0, 85.0), 0.5), "{:?}", bounds(&out));
    // Squashed to half height about its centre: the art follows, and is a plain path now.
    let squash = Affine::translate((50.0, 30.0)) * Affine::scale_non_uniform(1.0, 0.5) * Affine::translate((-50.0, -30.0));
    let to: Vec<Point> = m.rest.iter().map(|p| squash * *p).collect();
    let out = pose_node(&n, &m, &to).unwrap();
    assert!(near(bounds(&out), Rect::new(0.0, 15.0, 100.0, 45.0), 0.5), "{:?}", bounds(&out));
    assert!(matches!(&out.kind, NodeKind::Path { live: None, .. }), "a bent live shape is a plain path");
}

#[test]
fn groups_bend_member_by_member_and_bad_particles_are_refused() {
    let a = Arc::new(node(1, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0))));
    let b = Arc::new(node(2, shapes::rectangle(Rect::new(60.0, 0.0, 110.0, 50.0))));
    let g = Node::group(NodeId(3), vec![a, b]);
    let m = mesh(&g, Preset::Rubber, 8.0);
    let moved: Vec<Point> = m.rest.iter().map(|p| *p + vectorcraft_geom::Vec2::new(0.0, 100.0)).collect();
    let out = pose_node(&g, &m, &moved).unwrap();
    assert!(near(bounds(&out), Rect::new(0.0, 100.0, 110.0, 150.0), 0.5), "{:?}", bounds(&out));
    assert!(pose_node(&g, &m, &moved[1..]).is_none(), "a particle short");
    let mut nan = moved.clone();
    nan[0].x = f64::NAN;
    assert!(pose_node(&g, &m, &nan).is_none(), "not finite");
}
