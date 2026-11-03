//! PIC31.2: soft bodies from objects: cloth strips along the long axis, jelly cells with holes,
//! the caps, and junk outlines refused with a reason (never a panic).

use std::sync::Arc;

use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, shapes};

use crate::mesh::{MAX_CELLS_PER_BODY, MAX_PARTICLES_PER_BODY, MeshKind, soft_mesh};
use crate::shape::body_shape;
use crate::soft::{Preset, Soft};

fn node(id: u64, path: PathData) -> Node {
    Node::path(NodeId(id), path, Appearance::default_art())
}

fn ring(pts: &[(f64, f64)], closed: bool) -> PathData {
    PathData::single(SubPath::new(pts.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), closed))
}

fn preset(p: Preset, resolution: f64) -> Soft {
    Soft { resolution: Some(resolution), ..Soft::of(p) }
}

#[test]
fn a_cloth_strip_runs_along_the_long_axis_turned_with_the_object() {
    // A 200 × 10 rectangle turned 30°.
    let mut path = shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 10.0));
    path.transform(Affine::rotate_about(30f64.to_radians(), Point::new(100.0, 5.0)));
    let n = node(1, path);
    let m = soft_mesh(&body_shape(&n, None).unwrap(), &preset(Preset::Cloth, 10.0), 10_000).unwrap();
    let MeshKind::Strip(s) = &m.kind else { panic!("a strip") };
    assert!((s.length - 200.0).abs() < 0.5, "{s:?}");
    assert!((s.thickness - 10.0).abs() < 0.5, "{s:?}");
    let angle = s.axis.y.atan2(s.axis.x).to_degrees();
    assert!((angle - 30.0).abs() < 0.5, "{angle}");
    assert_eq!(m.rest.len(), 21);
    assert_eq!(m.segments.len(), 20);
    assert!((m.radius - 5.0).abs() < 0.5);
    // The centre line passes through the rectangle's centre.
    let mid = m.rest[10];
    assert!((mid - Point::new(100.0, 5.0)).hypot() < 0.5, "{mid:?}");
    // Every particle's offset across is 0, and along runs 0..length.
    for (i, p) in m.rest.iter().enumerate() {
        let (u, t) = s.local(*p);
        assert!(t.abs() < 1e-6 && (u - i as f64 * 10.0).abs() < 1e-6, "{i}: {u} {t}");
    }
}

#[test]
fn a_round_shape_makes_a_strip_along_its_wider_side() {
    let n = node(1, shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)));
    let m = soft_mesh(&body_shape(&n, None).unwrap(), &preset(Preset::Cloth, 5.0), 10_000).unwrap();
    let MeshKind::Strip(s) = &m.kind else { panic!("a strip") };
    assert!((s.axis.x - 1.0).abs() < 1e-9, "{s:?}");
}

#[test]
fn jelly_fills_the_outline_with_cells_and_keeps_holes() {
    // A 100 × 100 square with a 40 × 40 hole in the middle (a compound path).
    let mut path = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    let hole = shapes::rectangle(Rect::new(30.0, 30.0, 70.0, 70.0));
    path.subpaths.extend(hole.subpaths);
    let n = node(1, path);
    let m = soft_mesh(&body_shape(&n, None).unwrap(), &preset(Preset::Jelly, 10.0), 10_000).unwrap();
    assert_eq!(m.kind, MeshKind::Cells);
    assert!(m.rest.len() > 60 && m.rest.len() < 400, "{}", m.rest.len());
    assert!(!m.cells.is_empty() && !m.segments.is_empty());
    // No cell's centre lies in the hole, and the cells cover about the ring's area (8,400 pt²).
    let mut area = 0.0;
    for c in &m.cells {
        let [a, b, d] = c.map(|i| m.rest[i as usize]);
        let cen = Point::new((a.x + b.x + d.x) / 3.0, (a.y + b.y + d.y) / 3.0);
        assert!(!Rect::new(31.0, 31.0, 69.0, 69.0).contains(cen), "a cell in the hole at {cen:?}");
        area += ((b - a).cross(d - a) / 2.0).abs();
    }
    assert!((area - 8_400.0).abs() < 600.0, "{area}");
    // Every particle lies on or inside the outline.
    for p in &m.rest {
        assert!(Rect::new(-0.5, -0.5, 100.5, 100.5).contains(*p), "{p:?}");
    }
}

#[test]
fn two_separate_outlines_are_one_body_in_two_pieces() {
    let a = Arc::new(node(1, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0))));
    let b = Arc::new(node(2, shapes::ellipse(Rect::new(100.0, 0.0, 150.0, 50.0))));
    let g = Node::group(NodeId(3), vec![a, b]);
    let m = soft_mesh(&body_shape(&g, None).unwrap(), &preset(Preset::Rubber, 8.0), 10_000).unwrap();
    assert!(m.rest.iter().any(|p| p.x < 50.5) && m.rest.iter().any(|p| p.x > 99.5), "both pieces have particles");
}

#[test]
fn the_caps_coarsen_the_spacing_with_a_note() {
    let n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 2_000.0, 2_000.0)));
    let shape = body_shape(&n, None).unwrap();
    let m = soft_mesh(&shape, &preset(Preset::Jelly, 2.0), 1_000_000).unwrap();
    assert!(m.rest.len() <= MAX_PARTICLES_PER_BODY && m.cells.len() <= MAX_CELLS_PER_BODY, "{} {}", m.rest.len(), m.cells.len());
    assert!(m.spacing > 2.0 && !m.notes.is_empty(), "{m:?}");
    // The run's budget.
    let m = soft_mesh(&shape, &preset(Preset::Jelly, 20.0), 100).unwrap();
    assert!(m.rest.len() <= 100, "{}", m.rest.len());
    assert!(soft_mesh(&shape, &preset(Preset::Jelly, 20.0), 1).is_err(), "no budget left");
    let line = node(2, shapes::rectangle(Rect::new(0.0, 0.0, 100_000.0, 4.0)));
    let m = soft_mesh(&body_shape(&line, None).unwrap(), &preset(Preset::Cloth, 2.0), 1_000_000).unwrap();
    assert_eq!(m.rest.len(), MAX_PARTICLES_PER_BODY);
    assert!(!m.notes.is_empty());
}

#[test]
fn junk_outlines_are_refused_or_meshed_never_a_panic() {
    let junk: Vec<PathData> = vec![
        ring(&[(0.0, 0.0), (100.0, 0.0)], false),
        ring(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)], true),
        ring(&[(0.0, 0.0), (1.0, 1.0), (1.0, 0.0), (0.0, 1.0)], true),
        ring(&[(0.0, 0.0), (0.0, 0.0), (50.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0), (0.0, 50.0)], true),
        ring(&[(0.0, 0.0), (0.004, 0.0), (0.004, 0.004)], true),
        ring(&[(-900_000.0, -900_000.0), (900_000.0, -900_000.0), (900_000.0, 900_000.0)], true),
        ring(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (100.0, -500.0), (0.0, 100.0)], true),
        ring(&[(0.0, 0.0), (100.0, 0.0), (200.0, 1e-9)], true),
        ring(&[(0.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0)], true),
    ];
    for (k, path) in junk.into_iter().enumerate() {
        let n = node(k as u64, path);
        let Ok(shape) = body_shape(&n, None) else { continue };
        for p in [Preset::Cloth, Preset::Jelly, Preset::Rubber] {
            for res in [2.0, 12.0, 100.0] {
                match soft_mesh(&shape, &preset(p.clone(), res), 5_000) {
                    Ok(m) => {
                        assert!(m.rest.len() >= 2 && m.rest.len() <= MAX_PARTICLES_PER_BODY, "{k} {p:?}");
                        assert!(m.rest.iter().all(|q| q.x.is_finite() && q.y.is_finite()), "{k} {p:?}");
                        assert!(m.cells.iter().flatten().all(|i| (*i as usize) < m.rest.len()), "{k} {p:?}");
                    }
                    Err(e) => assert!(!e.is_empty()),
                }
            }
        }
    }
}

// PIC31.2a (review): outline groups, corners, the axis, budgets.

#[test]
fn a_group_member_on_another_is_not_a_hole_and_overlaps_merge() {
    // A small square on a big one, in a group: one filled body, no hole.
    let big = Arc::new(node(1, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0))));
    let small = Arc::new(node(2, shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0))));
    let g = Node::group(NodeId(3), vec![big, small]);
    let m = soft_mesh(&body_shape(&g, None).unwrap(), &preset(Preset::Jelly, 10.0), 10_000).unwrap();
    assert!(m.cells.iter().any(|c| {
        let [a, b, d] = c.map(|i| m.rest[i as usize]);
        Rect::new(40.0, 40.0, 60.0, 60.0).contains(Point::new((a.x + b.x + d.x) / 3.0, (a.y + b.y + d.y) / 3.0))
    }));
    assert!(!m.notes.is_empty(), "the merge is noted: {:?}", m.notes);
    // Two members overlapping: filled together, every particle inside their hull.
    let a = Arc::new(node(1, shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0))));
    let b = Arc::new(node(2, shapes::rectangle(Rect::new(40.0, 40.0, 100.0, 100.0))));
    let g = Node::group(NodeId(3), vec![a, b]);
    let m = soft_mesh(&body_shape(&g, None).unwrap(), &preset(Preset::Jelly, 10.0), 10_000).unwrap();
    assert!(m.rest.iter().any(|p| p.x > 90.0) && m.rest.iter().any(|p| p.x < 10.0));
    assert!(m.notes.iter().any(|n| n.contains("hull")), "{:?}", m.notes);
}

#[test]
fn separate_members_both_get_cells() {
    let a = Arc::new(node(1, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0))));
    let b = Arc::new(node(2, shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0))));
    let g = Node::group(NodeId(3), vec![a, b]);
    let m = soft_mesh(&body_shape(&g, None).unwrap(), &preset(Preset::Jelly, 10.0), 10_000).unwrap();
    let centres: Vec<f64> = m.cells.iter().map(|c| c.iter().map(|i| m.rest[*i as usize].x).sum::<f64>() / 3.0).collect();
    assert!(centres.iter().any(|x| *x < 50.0) && centres.iter().any(|x| *x > 100.0));
}

#[test]
fn corners_stay_corners() {
    let n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 90.0, 90.0)));
    let m = soft_mesh(&body_shape(&n, None).unwrap(), &preset(Preset::Rubber, 30.0), 10_000).unwrap();
    for c in [Point::new(0.0, 0.0), Point::new(90.0, 0.0), Point::new(90.0, 90.0), Point::new(0.0, 90.0)] {
        assert!(m.rest.iter().any(|p| (*p - c).hypot() < 1e-3), "corner {c:?} kept");
    }
}

#[test]
fn a_finely_flattened_short_side_doesnt_turn_the_strip() {
    // 200 wide, 150 tall; the short sides drawn with a hundred points each.
    let mut pts = vec![(0.0, 0.0), (200.0, 0.0)];
    pts.extend((1..100).map(|i| (200.0, i as f64 * 1.5)));
    pts.extend([(200.0, 150.0), (0.0, 150.0)]);
    pts.extend((1..100).rev().map(|i| (0.0, i as f64 * 1.5)));
    let n = node(1, ring(&pts, true));
    let m = soft_mesh(&body_shape(&n, None).unwrap(), &preset(Preset::Cloth, 10.0), 10_000).unwrap();
    let MeshKind::Strip(s) = &m.kind else { panic!("a strip") };
    assert!(s.axis.x.abs() > 0.99, "{s:?}");
}

#[test]
fn tiny_budgets_and_hand_made_junk_never_panic() {
    let n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)));
    let shape = body_shape(&n, None).unwrap();
    for cap in 0..5 {
        for p in [Preset::Jelly, Preset::Cloth] {
            if let Ok(m) = soft_mesh(&shape, &preset(p, 10.0), cap) {
                assert!(m.rest.len() <= cap.max(2));
            }
        }
    }
    let junk = crate::shape::BodyShape {
        pieces: vec![
            crate::shape::Piece::Ring { pts: vec![Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)], closed: true },
            crate::shape::Piece::Ring { pts: vec![], closed: true },
            crate::shape::Piece::Ball { center: Point::new(0.0, 0.0), radius: f64::INFINITY },
            crate::shape::Piece::Box(Rect::new(0.0, 0.0, 0.0, 0.0)),
        ],
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        notes: vec![],
        owners: vec![],
    };
    for p in [Preset::Jelly, Preset::Cloth] {
        if let Ok(m) = soft_mesh(&junk, &preset(p, 2.0), 1_000) {
            assert!(m.rest.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
        }
    }
    // Many small separate outlines: within the piece cap, quickly.
    let kids: Vec<Arc<Node>> = (0..200)
        .map(|i| {
            Arc::new(node(
                i,
                shapes::rectangle(Rect::new(
                    (i % 20) as f64 * 12.0,
                    (i / 20) as f64 * 12.0,
                    (i % 20) as f64 * 12.0 + 8.0,
                    (i / 20) as f64 * 12.0 + 8.0,
                )),
            ))
        })
        .collect();
    let g = Node::group(NodeId(999), kids);
    let t = std::time::Instant::now();
    let r = soft_mesh(&body_shape(&g, None).unwrap(), &preset(Preset::Jelly, 2.0), 1_200);
    assert!(t.elapsed().as_secs_f64() < 20.0, "{:?}", t.elapsed());
    if let Ok(m) = r {
        assert!(m.rest.len() <= 1_200 && !m.notes.is_empty());
    }
}
