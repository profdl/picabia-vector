//! PIC31.2b: the presets in a running world: each settles without blowing up, keeps its size,
//! and behaves as named (jelly wobbles more and rebounds less than rubber; cloth drapes).

use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{PathData, Point, Rect, Vec2, shapes};

use crate::mesh::soft_mesh;
use crate::pose::Pose;
use crate::settings::{BodyType, Settings};
use crate::shape::body_shape;
use crate::soft::{Preset, Soft};
use crate::world::{World, WorldBody, WorldParams};

const BOARD: Rect = Rect::new(0.0, 0.0, 600.0, 400.0);

fn soft(path: PathData, s: Soft) -> WorldBody {
    let n = Node::path(NodeId(1), path, Appearance::default_art());
    let shape = body_shape(&n, None).unwrap();
    let mesh = soft_mesh(&shape, &s, 5_000).unwrap();
    WorldBody { settings: Settings { body_type: BodyType::Soft, soft: Some(s), ..Settings::default() }, shape, mesh: Some(mesh), pins: vec![] }
}

fn params() -> WorldParams {
    WorldParams { gravity: Vec2::new(0.0, crate::sim::DEFAULT_GRAVITY), walls: Some(BOARD) }
}

fn particles(w: &World) -> Vec<Point> {
    match &w.poses()[0] {
        Pose::Soft(p) => p.clone(),
        other => panic!("{other:?}"),
    }
}

/// Drop a 100 × 80 ellipse of `preset` from near the top: (steps to settle, highest rebound above
/// the floor after first touching it (pt), largest squash: lowest height over rest height, area
/// kept at rest).
fn drop(preset: Preset) -> (u32, f64, f64, f64) {
    let body = soft(shapes::ellipse(Rect::new(250.0, 20.0, 350.0, 100.0)), Soft::of(preset));
    let mesh = body.mesh.clone().unwrap();
    let (mut w, _) = World::new(&[body], &params());
    let area = |p: &[Point]| {
        mesh.cells
            .iter()
            .map(|c| {
                let [a, b, d] = c.map(|i| p[i as usize]);
                ((b - a).cross(d - a) / 2.0).abs()
            })
            .sum::<f64>()
    };
    let (mut touched, mut rebound, mut squash) = (false, 0.0f64, f64::INFINITY);
    let mut steps = 0;
    while steps < 1_800 {
        w.step();
        steps += 1;
        let p = particles(&w);
        assert!(p.iter().all(|q| q.x.is_finite() && q.y.is_finite()), "finite");
        let (top, bottom) = p.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(t, b), q| (t.min(q.y), b.max(q.y)));
        if bottom > BOARD.y1 - 2.0 {
            touched = true;
        }
        if touched {
            rebound = rebound.max(BOARD.y1 - bottom);
            squash = squash.min((bottom - top) / 80.0);
        }
        if steps > 60 && w.settled() {
            break;
        }
    }
    let kept = area(&particles(&w)) / area(&mesh.rest);
    (steps, rebound, squash, kept)
}

#[test]
fn presets_settle_keep_their_size_and_behave_as_named() {
    let jelly = drop(Preset::Jelly);
    let rubber = drop(Preset::Rubber);
    println!("jelly (steps, rebound pt, squash, area kept) {jelly:?}");
    println!("rubber {rubber:?}");
    for (name, (steps, _, squash, kept)) in [("jelly", jelly), ("rubber", rubber)] {
        assert!(steps < 1_800, "{name} settles within 30 s: {steps}");
        assert!((kept - 1.0).abs() < 0.15, "{name} keeps its area: {kept}");
        assert!(squash > 0.4, "{name} doesn't collapse: {squash}");
    }
    assert!(jelly.2 < rubber.2, "jelly squashes more than rubber: {} vs {}", jelly.2, rubber.2);
}

#[test]
fn cloth_hangs_from_its_ends_and_sags() {
    let mut cloth = soft(shapes::rectangle(Rect::new(100.0, 100.0, 400.0, 104.0)), Soft::of(Preset::Cloth));
    let n = cloth.mesh.as_ref().unwrap().rest.len() as u32;
    cloth.pins = vec![(0, None), (n - 1, None)];
    let (mut w, _) = World::new(&[cloth], &params());
    for _ in 0..600 {
        w.step();
    }
    let p = particles(&w);
    let sag = p[p.len() / 2].y - 102.0;
    // A chain between two points 300 apart: low stretch keeps the sag modest, low bend lets it sag.
    println!("cloth sag {sag:.1} pt, length now {:.1}", p.windows(2).map(|w| (w[1] - w[0]).hypot()).sum::<f64>());
    assert!(sag > 10.0 && sag < 150.0, "{sag}");
    let length: f64 = p.windows(2).map(|w| (w[1] - w[0]).hypot()).sum();
    assert!(length < 300.0 * 1.15, "stretches little: {length}");
}
