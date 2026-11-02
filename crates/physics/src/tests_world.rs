//! PIC31.4: the live world and its clock: soft and rigid bodies together, the same bits when a
//! session is replayed from a snapshot, pokes recorded and replayed, retuning and restarts.

use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{PathData, Point, Rect, Vec2, shapes};

use crate::mesh::soft_mesh;
use crate::pose::Pose;
use crate::session::{Event, MAX_EVENTS, SimSession};
use crate::settings::{BodyType, Settings};
use crate::shape::body_shape;
use crate::soft::{Pin, Preset, Soft};
use crate::world::{World, WorldBody, WorldParams};

const BOARD: Rect = Rect::new(0.0, 0.0, 600.0, 400.0);

fn node(id: u64, path: PathData) -> Node {
    Node::path(NodeId(id), path, Appearance::default_art())
}

fn rigid(r: Rect, t: BodyType) -> WorldBody {
    let n = node(1, shapes::rectangle(r));
    WorldBody { settings: Settings { body_type: t, ..Settings::default() }, shape: body_shape(&n, None).unwrap(), mesh: None, pins: vec![] }
}

fn soft(path: PathData, s: Soft) -> WorldBody {
    let n = node(2, path);
    let shape = body_shape(&n, None).unwrap();
    let mesh = soft_mesh(&shape, &s, 5_000).unwrap();
    WorldBody { settings: Settings { body_type: BodyType::Soft, soft: Some(s), ..Settings::default() }, shape, mesh: Some(mesh), pins: vec![] }
}

fn params() -> WorldParams {
    WorldParams { gravity: Vec2::new(0.0, crate::sim::DEFAULT_GRAVITY), walls: Some(BOARD) }
}

fn particles(w: &World, i: usize) -> Vec<Point> {
    match &w.poses()[i] {
        Pose::Soft(p) => p.clone(),
        other => panic!("not soft: {other:?}"),
    }
}

fn bits(w: &World) -> Vec<u64> {
    w.poses()
        .iter()
        .flat_map(|p| match p {
            Pose::Rigid(a) => a.map_or_else(Vec::new, |a| a.as_coeffs().to_vec()),
            Pose::Soft(ps) => ps.iter().flat_map(|q| [q.x, q.y]).collect(),
        })
        .map(f64::to_bits)
        .collect()
}

#[test]
fn a_jelly_falls_rests_on_the_floor_and_keeps_its_size() {
    let body = soft(shapes::ellipse(Rect::new(250.0, 50.0, 350.0, 130.0)), Soft::of(Preset::Jelly));
    let rest = body.mesh.clone().unwrap();
    let (mut w, _) = World::new(&[body], &params());
    let mut steps = 0;
    while steps < 1_200 && !(steps > 60 && w.settled()) {
        w.step();
        steps += 1;
    }
    let p = particles(&w, 0);
    let bottom = p.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max);
    assert!((bottom - BOARD.y1).abs() < 8.0, "rests on the floor: {bottom}");
    // Area kept within a fifth (no collapse, no blow-up).
    let area = |pts: &[Point]| {
        rest.cells
            .iter()
            .map(|c| {
                let [a, b, d] = c.map(|i| pts[i as usize]);
                ((b - a).cross(d - a) / 2.0).abs()
            })
            .sum::<f64>()
    };
    let (a0, a1) = (area(&rest.rest), area(&p));
    assert!((a1 / a0 - 1.0).abs() < 0.2, "{a0} → {a1}");
    assert!(p.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
}

#[test]
fn cloth_drapes_over_a_static_shelf() {
    let shelf = rigid(Rect::new(250.0, 200.0, 350.0, 210.0), BodyType::Static);
    let cloth = soft(shapes::rectangle(Rect::new(150.0, 150.0, 450.0, 154.0)), Soft::of(Preset::Cloth));
    let (mut w, _) = World::new(&[shelf, cloth], &params());
    for _ in 0..240 {
        w.step();
    }
    let p = particles(&w, 1);
    let mid = p[p.len() / 2];
    let (first, last) = (p[0], p[p.len() - 1]);
    assert!(mid.y < 205.0 && mid.y > 185.0, "the middle rests on the shelf: {mid:?}");
    assert!(first.y > 230.0 && last.y > 230.0, "the ends hang down: {first:?} {last:?}");
}

#[test]
fn rigid_bodies_move_as_in_phase_1() {
    let bodies = [rigid(Rect::new(100.0, 20.0, 160.0, 60.0), BodyType::Dynamic), rigid(Rect::new(300.0, 40.0, 340.0, 120.0), BodyType::Dynamic)];
    let (mut w, _) = World::new(&bodies, &params());
    for _ in 0..200 {
        w.step();
    }
    let inputs: Vec<crate::BodyInput> = bodies.iter().map(|b| crate::BodyInput { settings: b.settings.clone(), shape: b.shape.clone() }).collect();
    let phase1 =
        crate::simulate(&inputs, &crate::SimParams { walls: Some(BOARD), max_steps: 200, stop_when_settled: false, ..crate::SimParams::default() });
    for (pose, o) in w.poses().iter().zip(&phase1.outcomes) {
        assert_eq!(*pose, Pose::Rigid(o.moved), "the same bits as phase 1's drop");
    }
}

#[test]
fn seeking_back_replays_the_same_bits_with_pokes() {
    let bodies = [
        rigid(Rect::new(100.0, 20.0, 160.0, 60.0), BodyType::Dynamic),
        soft(shapes::ellipse(Rect::new(250.0, 50.0, 330.0, 110.0)), Soft::of(Preset::Rubber)),
        soft(shapes::rectangle(Rect::new(380.0, 100.0, 560.0, 104.0)), Soft::of(Preset::Cloth)),
    ];
    let (w, _) = World::new(&bodies, &params());
    let mut s = SimSession::new(w);
    s.advance(50);
    assert!(s.record(Event::Grab { body: 1, at: Point::new(290.0, 80.0) }));
    s.advance(1);
    for k in 0..20 {
        s.record(Event::DragTo { to: Point::new(290.0 + k as f64 * 5.0, 60.0) });
        s.advance(1);
    }
    s.record(Event::Release);
    s.advance(200);
    let end = bits(s.world());
    let at = s.step();
    // Back to the start, forward again: the pokes replay.
    s.seek(0);
    s.seek(at);
    assert_eq!(bits(s.world()), end);
    // Back to a snapshot past the poke, forward again.
    s.seek(130);
    s.seek(at);
    assert_eq!(bits(s.world()), end);
    // Recording after seeking back drops what came after.
    s.seek(40);
    s.record(Event::Release);
    assert!(s.events().iter().all(|(step, _)| *step <= 40));
}

#[test]
fn pulls_in_one_step_coalesce_and_events_are_capped() {
    let (w, _) = World::new(&[rigid(Rect::new(100.0, 20.0, 160.0, 60.0), BodyType::Dynamic)], &params());
    let mut s = SimSession::new(w);
    s.record(Event::Grab { body: 0, at: Point::new(130.0, 40.0) });
    for k in 0..10 {
        s.record(Event::DragTo { to: Point::new(130.0 + f64::from(k), 40.0) });
    }
    assert_eq!(s.events().len(), 2);
    let mut refused = false;
    for k in 0..MAX_EVENTS + 5 {
        if !s.record(if k % 2 == 0 { Event::Release } else { Event::Grab { body: 0, at: Point::ZERO } }) {
            refused = true;
            break;
        }
    }
    assert!(refused && s.events().len() == MAX_EVENTS);
}

#[test]
fn a_poke_pulls_a_body_and_letting_go_throws_it() {
    let (mut w, _) = World::new(&[soft(shapes::ellipse(Rect::new(250.0, 300.0, 330.0, 380.0)), Soft::of(Preset::Rubber))], &params());
    for _ in 0..60 {
        w.step();
    }
    let before = particles(&w, 0);
    let c0 = before.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / before.len() as f64;
    assert!(w.grab(0, c0.to_point()));
    w.drag_to(Point::new(c0.x + 150.0, c0.y - 150.0));
    for _ in 0..60 {
        w.step();
    }
    let after = particles(&w, 0);
    let c1 = after.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / after.len() as f64;
    assert!(c1.x - c0.x > 40.0 && c0.y - c1.y > 40.0, "pulled up and right: {c0:?} → {c1:?}");
    w.release();
    assert!(w.grabbed().is_none());
    // A static body can't be grabbed; an unknown one neither.
    let (mut w2, _) = World::new(&[rigid(Rect::new(0.0, 0.0, 10.0, 10.0), BodyType::Static)], &params());
    assert!(!w2.grab(0, Point::new(5.0, 5.0)) && !w2.grab(7, Point::ZERO) && !w2.grab(0, Point::new(f64::NAN, 0.0)));
}

#[test]
fn pinned_ends_stay_and_attached_points_ride_their_body() {
    let mut cloth = soft(shapes::rectangle(Rect::new(100.0, 100.0, 400.0, 104.0)), Soft::of(Preset::Cloth));
    let n = cloth.mesh.as_ref().unwrap().rest.len() as u32;
    cloth.pins = vec![(0, None), (n - 1, Some(1))];
    let carrier = rigid(Rect::new(380.0, 60.0, 420.0, 90.0), BodyType::Dynamic);
    let (mut w, _) = World::new(&[cloth, carrier], &params());
    let start = particles(&w, 0);
    for _ in 0..120 {
        w.step();
    }
    let p = particles(&w, 0);
    assert!((p[0] - start[0]).hypot() < 1e-3, "pinned in place: {:?} → {:?}", start[0], p[0]);
    assert!(p[(n - 1) as usize].y > start[(n - 1) as usize].y + 20.0, "attached to the falling box: {:?}", p[(n - 1) as usize]);
    // The pin unit stores places on the object.
    let _ = Pin::new(0.0, 0.5, None);
}

#[test]
fn retuning_and_restarting_take_effect_and_junk_is_harmless() {
    let mut body = soft(shapes::ellipse(Rect::new(250.0, 50.0, 330.0, 110.0)), Soft::of(Preset::Jelly));
    let (mut w, _) = World::new(&[body.clone()], &params());
    for _ in 0..30 {
        w.step();
    }
    let mut stiff = body.settings.clone();
    stiff.soft.as_mut().unwrap().stiffness = Some(1.0);
    w.retune(0, &stiff);
    w.retune(5, &stiff);
    w.step();
    // Restart: back at rest.
    body.settings = stiff;
    w.restart_body(0, &body);
    let back = particles(&w, 0);
    assert!(back.iter().zip(&body.mesh.as_ref().unwrap().rest).all(|(a, b)| (*a - *b).hypot() < 1e-3), "back at rest (f32 world)");
    w.restart_body(9, &body);
    // Junk worlds.
    let (mut empty, _) =
        World::new(&[], &WorldParams { gravity: Vec2::new(f64::NAN, f64::INFINITY), walls: Some(Rect::new(0.0, 0.0, f64::NAN, 1.0)) });
    empty.step();
    assert!(empty.poses().is_empty() && empty.settled());
}
