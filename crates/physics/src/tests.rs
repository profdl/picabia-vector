use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, Vec2, shapes};

use crate::settings::{BodyType, ColliderType, Settings};
use crate::shape::{body_shape, convex_parts, simplify};
use crate::sim::{BodyInput, SimParams, simulate};

const BOARD: Rect = Rect::new(0.0, 0.0, 400.0, 300.0);

fn node(id: u64, path: PathData) -> Node {
    Node::path(NodeId(id), path, Appearance::default_art())
}

fn body(n: &Node, s: Settings) -> BodyInput {
    BodyInput { settings: s, shape: body_shape(n, None).unwrap() }
}

fn walls() -> SimParams {
    SimParams { walls: Some(BOARD), ..SimParams::default() }
}

fn moved_rect(r: Rect, a: Option<vectorcraft_geom::Affine>) -> Rect {
    let a = a.unwrap_or_default();
    let pts = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].map(|p| a * p);
    pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

/// The bounds of `b`'s outline points moved by `a`.
fn moved_outline(b: &BodyInput, a: Option<vectorcraft_geom::Affine>) -> Rect {
    let a = a.unwrap_or_default();
    let pts: Vec<Point> = b.shape.all_points().into_iter().map(|p| a * p).collect();
    pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

#[test]
fn a_box_falls_onto_the_floor_and_rests_at_the_bottom() {
    let r = Rect::new(150.0, 20.0, 210.0, 60.0);
    let n = node(1, shapes::rectangle(r));
    let out = simulate(&[body(&n, Settings::default())], &walls());
    assert!(out.settled, "{out:?}");
    assert!(out.steps < 3_000);
    let end = moved_rect(r, out.outcomes[0].moved);
    assert!((end.y1 - BOARD.y1).abs() < 1.0, "bottom at {}", end.y1);
    assert!((end.width() - 60.0).abs() < 0.5 && (end.height() - 40.0).abs() < 0.5, "turned: {end:?}");
    // A bounce or two can nudge it sideways a little.
    assert!((end.center().x - 180.0).abs() < 3.0, "slid: {end:?}");
}

#[test]
fn a_ball_bounces_higher_with_restitution() {
    let r = Rect::new(180.0, 20.0, 220.0, 60.0);
    let n = node(1, shapes::ellipse(r));
    // The highest the ball gets after its first bounce, in steps of a fixed run.
    let peak = |restitution: f64| {
        let s = Settings { restitution, ..Settings::default() };
        let b = body(&n, s);
        // Run to just past the first impact, then measure how far it rises again.
        let mut lowest = f64::MIN;
        let mut after = f64::MAX;
        for steps in (10..=200).step_by(5) {
            let out = simulate(std::slice::from_ref(&b), &SimParams { max_steps: steps, stop_when_settled: false, ..walls() });
            let y = moved_rect(r, out.outcomes[0].moved).y1;
            if y >= lowest {
                lowest = y;
            } else {
                after = after.min(y);
            }
        }
        BOARD.y1 - after.min(BOARD.y1)
    };
    let (dead, bouncy) = (peak(0.0), peak(1.0));
    assert!(bouncy > 40.0, "bounced {bouncy}");
    // The walls' restitution is 0.3 and the rule takes the larger: a dead ball still bounces a bit.
    assert!(bouncy > dead * 2.0, "dead {dead}, bouncy {bouncy}");
}

#[test]
fn a_static_body_stays_and_holds_what_falls_on_it() {
    let shelf = Rect::new(100.0, 150.0, 300.0, 170.0);
    let boxr = Rect::new(170.0, 40.0, 230.0, 80.0);
    let s = node(1, shapes::rectangle(shelf));
    let b = node(2, shapes::rectangle(boxr));
    let still = Settings { body_type: BodyType::Static, ..Settings::default() };
    let out = simulate(&[body(&s, still), body(&b, Settings::default())], &walls());
    assert_eq!(out.outcomes[0].moved, None);
    let end = moved_rect(boxr, out.outcomes[1].moved);
    assert!((end.y1 - shelf.y0).abs() < 1.0, "rests at {}", end.y1);
}

#[test]
fn kinematic_and_unknown_bodies_stay_put() {
    let r = Rect::new(150.0, 20.0, 210.0, 60.0);
    let n = node(1, shapes::rectangle(r));
    for t in [BodyType::Kinematic, BodyType::Other("inflatable".into())] {
        let out = simulate(&[body(&n, Settings { body_type: t, ..Settings::default() })], &walls());
        assert_eq!(out.outcomes[0].moved, None);
    }
}

#[test]
fn a_sensor_passes_through_the_floor() {
    let r = Rect::new(150.0, 20.0, 210.0, 60.0);
    let n = node(1, shapes::rectangle(r));
    let out = simulate(&[body(&n, Settings { sensor: true, ..Settings::default() })], &SimParams { max_steps: 120, ..walls() });
    let end = moved_rect(r, out.outcomes[0].moved);
    assert!(end.y0 > BOARD.y1, "still above the floor: {end:?}");
}

#[test]
fn without_walls_a_body_falls_out_of_the_world_and_stays() {
    let r = Rect::new(150.0, 20.0, 210.0, 60.0);
    let n = node(1, shapes::rectangle(r));
    let p = SimParams { gravity: Vec2::new(0.0, 100_000.0), max_steps: 20_000, ..SimParams::default() };
    let out = simulate(&[body(&n, Settings::default())], &p);
    assert!(out.outcomes[0].lost && out.outcomes[0].moved.is_none(), "{out:?}");
    assert!(out.settled && out.steps < 20_000, "{out:?}");
}

#[test]
fn gravity_sideways_piles_against_the_right_wall() {
    let r = Rect::new(150.0, 120.0, 210.0, 160.0);
    let n = node(1, shapes::rectangle(r));
    let p = SimParams { gravity: Vec2::new(981.0, 0.0), ..walls() };
    let end = moved_rect(r, simulate(&[body(&n, Settings::default())], &p).outcomes[0].moved);
    assert!((end.x1 - BOARD.x1).abs() < 1.0, "{end:?}");
}

#[test]
fn twelve_shapes_settle_inside_the_artboard() {
    let mut bodies = vec![];
    for i in 0..12u32 {
        let x = 30.0 + f64::from(i % 4) * 90.0;
        let y = 10.0 + f64::from(i / 4) * 60.0;
        let r = Rect::new(x, y, x + 40.0 + f64::from(i % 3) * 10.0, y + 30.0);
        let path = match i % 3 {
            0 => shapes::rectangle(r),
            1 => shapes::ellipse(r),
            _ => shapes::star(r.center(), 25.0, 12.0, 5, 0.0),
        };
        let n = node(i.into(), path);
        bodies.push(body(&n, Settings::default()));
    }
    let out = simulate(&bodies, &walls());
    assert!(out.settled, "{out:?}");
    for (b, o) in bodies.iter().zip(&out.outcomes) {
        let end = moved_outline(b, o.moved);
        assert!(end.y1 <= BOARD.y1 + 2.0 && end.x0 >= -2.0 && end.x1 <= BOARD.x1 + 2.0, "{end:?}");
        assert!(o.moved.is_some());
    }
}

#[test]
fn the_same_input_gives_the_same_bits() {
    let mut bodies = vec![];
    for i in 0..8u32 {
        let x = 20.0 + f64::from(i) * 45.0;
        let n = node(i.into(), shapes::star(Point::new(x, 40.0 + f64::from(i % 2) * 30.0), 20.0, 9.0, 5, f64::from(i) * 7.0));
        bodies.push(body(&n, Settings { collider: ColliderType::Precise, ..Settings::default() }));
    }
    let a = simulate(&bodies, &walls());
    let b = simulate(&bodies, &walls());
    let bits = |r: &crate::SimResult| r.outcomes.iter().flat_map(|o| o.moved.unwrap_or_default().as_coeffs()).map(f64::to_bits).collect::<Vec<_>>();
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a.steps, b.steps);
    // A fingerprint of the run, printed for comparing native and wasm builds.
    let h = bits(&a).iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b).wrapping_mul(0x100_0000_01b3));
    println!("physics fingerprint {h:016x} steps {}", a.steps);
    // Pinned (PIC31.0a): rigid-only scenes keep phase 1's guarantee, the same bits on every
    // platform and in wasm32 (checked under Node in PIC30.1). Soft bodies (PIC31) must leave
    // this hash alone; a rapier upgrade that changes it is a change of results to look into.
    assert_eq!((h, a.steps), (0x6f8e_17c7_d634_739d, 109), "the rigid-only fingerprint changed");
}

#[test]
fn every_fidelity_builds_and_settles() {
    let r = Rect::new(150.0, 20.0, 230.0, 80.0);
    let n = node(1, shapes::star(r.center(), 40.0, 15.0, 7, 0.0));
    for c in [ColliderType::Auto, ColliderType::Precise, ColliderType::Fast, ColliderType::Box] {
        for t in [BodyType::Dynamic, BodyType::Static] {
            let out = simulate(&[body(&n, Settings { collider: c, body_type: t.clone(), ..Settings::default() })], &walls());
            assert!(out.settled, "{c:?} {t:?}");
        }
    }
}

#[test]
fn an_open_path_and_a_group_fall_as_one_body() {
    let line = node(1, PathData::single(SubPath::new(vec![Anchor::corner(Point::new(100.0, 30.0)), Anchor::corner(Point::new(200.0, 50.0))], false)));
    let out = simulate(&[body(&line, Settings::default())], &walls());
    assert!(out.settled);
    let end = moved_rect(Rect::new(100.0, 30.0, 200.0, 50.0), out.outcomes[0].moved);
    assert!(end.y1 > 250.0, "{end:?}");

    let a = Arc::new(node(2, shapes::rectangle(Rect::new(100.0, 10.0, 140.0, 50.0))));
    let b = Arc::new(node(3, shapes::ellipse(Rect::new(160.0, 10.0, 200.0, 50.0))));
    let g = Node::group(NodeId(4), vec![a, b]);
    let out = simulate(&[body(&g, Settings::default())], &walls());
    assert!(out.settled);
    let end = moved_rect(g.geometric_bounds().unwrap(), out.outcomes[0].moved);
    assert!((end.y1 - BOARD.y1).abs() < 2.0, "{end:?}");
}

#[test]
fn junk_settings_clamp_with_warnings_and_keep_unknown_fields() {
    let (s, w) = crate::parse(
        r#"{"bodyType":"inflatable","colliderType":"weird","gravityScale":9,"restitution":-1,"friction":"x","mass":1e9,"angularDamping":null,"linearDamping":25,"sensor":3,"future":{"a":1}}"#,
    )
    .unwrap();
    assert_eq!(s.body_type, BodyType::Other("inflatable".into()));
    assert_eq!(s.collider, ColliderType::Auto);
    assert_eq!((s.gravity_scale, s.restitution, s.friction, s.mass, s.linear_damping), (2.0, 0.0, 0.4, 10_000.0, 20.0));
    assert!(!s.sensor);
    assert!(w.len() >= 8, "{w:?}");
    // Round trip keeps the unknown kind and field.
    let (again, _) = crate::parse(&s.to_json()).unwrap();
    assert_eq!(again, s);
    assert_eq!(again.extra.get("future"), Some(&json!({"a": 1})));
    assert!(crate::parse("[1]").is_err() && crate::parse("{").is_err() && crate::parse(&" ".repeat(20_000)).is_err());
    let (old, _) = crate::parse(r#"{"gravity":0.5}"#).unwrap();
    assert_eq!(old.gravity_scale, 0.5);
    assert!(!old.extra.contains_key("gravity"));
}

#[test]
fn settings_live_in_the_foreign_attribute() {
    let mut n = node(1, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)));
    assert!(crate::node_settings(&n).is_none());
    let s = Settings { body_type: BodyType::Static, ..Settings::default() };
    crate::set_node_settings(&mut n, Some(&s));
    assert_eq!(crate::node_settings(&n).unwrap().unwrap().0, s);
    crate::set_node_settings(&mut n, None);
    assert!(!crate::has_physics(&n));
}

#[test]
fn junk_geometry_is_refused_or_survives_never_panics() {
    let nan = node(
        1,
        PathData::single(SubPath::new(
            [(f64::NAN, 0.0), (10.0, f64::INFINITY), (0.0, 10.0)].map(|(x, y)| Anchor::corner(Point::new(x, y))).to_vec(),
            true,
        )),
    );
    assert!(body_shape(&nan, None).is_err());
    let huge = node(2, shapes::rectangle(Rect::new(0.0, 0.0, 1.0e9, 10.0)));
    assert!(body_shape(&huge, None).is_err());
    let empty = node(3, PathData::new(vec![]));
    assert!(body_shape(&empty, None).is_err());
    let dot = node(4, shapes::rectangle(Rect::new(5.0, 5.0, 5.0, 5.0)));
    assert!(body_shape(&dot, None).is_err());
    let empty_group = Node::group(NodeId(5), vec![]);
    assert!(body_shape(&empty_group, None).is_err());
    // A hairline, a sliver, a bow tie and a many-pointed path all simulate.
    let hair = node(6, shapes::rectangle(Rect::new(100.0, 10.0, 300.0, 10.0)));
    let sliver = node(7, shapes::rectangle(Rect::new(100.0, 20.0, 300.0, 20.001)));
    let bow = node(
        8,
        PathData::single(SubPath::new(
            [(100.0, 50.0), (200.0, 90.0), (200.0, 50.0), (100.0, 90.0)].map(|(x, y)| Anchor::corner(Point::new(x, y))).to_vec(),
            true,
        )),
    );
    let many: Vec<Anchor> = (0..20_000)
        .map(|i| {
            let a = f64::from(i) * 0.000_314_159_265;
            let r = if i % 2 == 0 { 60.0 } else { 40.0 };
            Anchor::corner(Point::new(200.0 + r * a.cos(), 150.0 + r * a.sin()))
        })
        .collect();
    let many = node(9, PathData::single(SubPath::new(many, true)));
    let mut bodies = vec![];
    for n in [&hair, &sliver, &bow, &many] {
        let shape = body_shape(n, None).unwrap();
        assert!(shape.all_points().len() <= crate::shape::MAX_POINTS_PER_BODY + 64);
        for c in [ColliderType::Auto, ColliderType::Precise, ColliderType::Fast, ColliderType::Box] {
            bodies.push(BodyInput { settings: Settings { collider: c, ..Settings::default() }, shape: shape.clone() });
        }
    }
    let out = simulate(&bodies, &SimParams { max_steps: 200, ..walls() });
    assert_eq!(out.outcomes.len(), bodies.len());
    // Junk run parameters are clamped.
    let p = SimParams {
        gravity: Vec2::new(f64::NAN, f64::INFINITY),
        walls: Some(Rect::new(f64::NAN, 0.0, 1.0, 1.0)),
        max_steps: u32::MAX,
        stop_when_settled: true,
    };
    let out = simulate(&bodies[..1], &p);
    assert!(out.steps <= crate::sim::MAX_STEPS);
}

#[test]
fn convex_parts_cover_an_l_shape() {
    let l = [(0.0, 0.0), (30.0, 0.0), (30.0, 10.0), (10.0, 10.0), (10.0, 30.0), (0.0, 30.0)].map(|(x, y)| Point::new(x, y));
    let parts = convex_parts(&l).unwrap();
    assert!(parts.len() >= 2 && parts.len() <= 4, "{parts:?}");
    let area: f64 = parts.iter().map(|p| crate::shape::signed_area2(p).abs() / 2.0).sum();
    assert!((area - 500.0).abs() < 1e-6, "{area}");
    assert!(simplify(&l, 4, true).len() <= 4);
}
