//! The drop-and-settle simulation: build a rapier2d world from bodies (settings + outline), step
//! it at 60 Hz until every dynamic body sleeps or a step cap, and return each body's rigid motion
//! as a document-space affine.
//!
//! No clock and no wall time: the only limits are step counts, so a run is the same on every
//! machine and on the web (rapier's `enhanced-determinism`: same input, same bits).

use rapier2d::prelude::*;
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::settings::{BodyType, ColliderType, Settings};
use crate::shape::{BodyShape, Piece, convex_hull, convex_parts, signed_area2, simplify};
use crate::{PT_PER_M, WALL_FRICTION, WALL_RESTITUTION, WALL_THICKNESS};

/// The step rate (tlprocedural's `targetUpdateRate`).
pub const HZ: f64 = 60.0;
/// Steps run when the caller gives no cap.
pub const DEFAULT_MAX_STEPS: u32 = 3_000;
/// The largest step cap.
pub const MAX_STEPS: u32 = 20_000;
/// The most bodies one run takes.
pub const MAX_BODIES: usize = 1_000;
/// Bodies × steps one run may do; a bigger cap is lowered (with a note).
pub const MAX_BODY_STEPS: u64 = 6_000_000;
/// A dynamic body this far (points) from the scene's centre has fallen out of the world: it is
/// taken out and left where it was.
pub const LOST_DISTANCE: f64 = 1.0e6;
/// Default gravity, points per second² down the page (9.81 m/s² at [`PT_PER_M`]).
pub const DEFAULT_GRAVITY: f64 = 9.81 * PT_PER_M;
/// The largest gravity component, points per second².
pub const MAX_GRAVITY: f64 = 1.0e5;
/// The most collider pieces (convex parts, edges' polylines, boxes, balls) in one run; a body
/// that would go over collides as its bounding box (with a note), so the step caps bound the time.
/// Overlapping compounds make a contact manifold per pair of touching parts, and rapier's
/// contact graph holds at most 2²² (4,194,304) of them per bucket (a debug assertion, a corrupt
/// graph without one). Bodies past the budget still take one box each, so a run has at most
/// 1,800 + [`MAX_BODIES`] = 2,800 pieces: under 4 million pairs even all piled on one spot.
pub const MAX_PIECES_PER_RUN: usize = 1_800;
/// The most points one exactly split ring keeps (more are simplified first).
const MAX_SPLIT_POINTS: usize = 256;
/// The smallest half extent of a box collider, metres (so a hairline still has a box).
const MIN_HALF: f32 = 0.0005;

/// One body to simulate.
#[derive(Clone, Debug)]
pub struct BodyInput {
    pub settings: Settings,
    pub shape: BodyShape,
}

/// The run's parameters.
#[derive(Clone, Debug)]
pub struct SimParams {
    /// Points per second², document axes (y down the page).
    pub gravity: Vec2,
    /// Walls around this rectangle (the artboard), or none.
    pub walls: Option<Rect>,
    /// The step cap, 1–[`MAX_STEPS`].
    pub max_steps: u32,
    /// Stop as soon as every dynamic body sleeps (else run all `max_steps`).
    pub stop_when_settled: bool,
}

impl Default for SimParams {
    fn default() -> Self {
        SimParams { gravity: Vec2::new(0.0, DEFAULT_GRAVITY), walls: None, max_steps: DEFAULT_MAX_STEPS, stop_when_settled: true }
    }
}

/// What happened to one body.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyOutcome {
    /// The body's rigid motion in document space (rotation then translation), `None` when it
    /// didn't move (static, kinematic, unmoved, lost).
    pub moved: Option<Affine>,
    /// It fell out of the world ([`LOST_DISTANCE`]) and was left where it was.
    pub lost: bool,
}

/// A run's result: one outcome per input body, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct SimResult {
    pub outcomes: Vec<BodyOutcome>,
    pub steps: u32,
    /// Every dynamic body (sensors and lost ones aside) was asleep when it stopped.
    pub settled: bool,
    pub notes: Vec<String>,
}

fn v(p: Point, origin: Point) -> Vector {
    Vector::new(((p.x - origin.x) / PT_PER_M) as f32, ((p.y - origin.y) / PT_PER_M) as f32)
}

fn fin(x: f64) -> f32 {
    if x.is_finite() { x as f32 } else { 0.0 }
}

/// Run the simulation. Never panics on any input: out-of-range values are clamped, bodies over
/// [`MAX_BODIES`] are left out with a note.
pub fn simulate(bodies: &[BodyInput], params: &SimParams) -> SimResult {
    let mut notes = Vec::new();
    let take = bodies.len().min(MAX_BODIES);
    if bodies.len() > take {
        notes.push(format!("{} bodies (at most {MAX_BODIES}): the last {} were left out", bodies.len(), bodies.len() - take));
    }
    let mut max_steps = params.max_steps.clamp(1, MAX_STEPS);
    let budget = (MAX_BODY_STEPS / (take.max(1) as u64)).max(1);
    if u64::from(max_steps) > budget {
        max_steps = u32::try_from(budget).unwrap_or(MAX_STEPS).max(1);
        notes.push(format!("{take} bodies: at most {max_steps} steps"));
    }
    let g = |x: f64| if x.is_finite() { x.clamp(-MAX_GRAVITY, MAX_GRAVITY) } else { 0.0 };
    let gravity = Vector::new(fin(g(params.gravity.x) / PT_PER_M), fin(g(params.gravity.y) / PT_PER_M));

    // The world's origin: the walls' centre, else the bodies' (small numbers keep f32 precise).
    let origin = params
        .walls
        .filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|x| x.is_finite()))
        .map(|r| r.center())
        .or_else(|| bodies.iter().take(take).map(|b| b.shape.bounds).reduce(|a, b| a.union(b)).map(|r| r.center()))
        .unwrap_or(Point::ZERO);

    let mut rbs = RigidBodySet::new();
    let mut cols = ColliderSet::new();
    let mut handles: Vec<Option<RigidBodyHandle>> = Vec::with_capacity(take);
    let mut pieces_left = MAX_PIECES_PER_RUN;
    let mut boxed = 0usize;
    for b in bodies.iter().take(take) {
        handles.push(add_body(&mut rbs, &mut cols, b, origin, &mut pieces_left, &mut boxed));
    }
    if boxed > 0 {
        notes.push(format!("over {MAX_PIECES_PER_RUN} collider pieces in all: {boxed} bodies collide as their bounding boxes"));
    }
    if let Some(r) = params.walls {
        add_walls(&mut rbs, &mut cols, r, origin);
    }

    // A falling body moves up to ~15 pt a step at artboard heights: a 20 pt contact margin (0.2 m)
    // catches it before it sinks into what it lands on (rapier's default, 2 pt, let a dropped box
    // sink and kick out spinning).
    let integration = IntegrationParameters { dt: (1.0 / HZ) as f32, normalized_prediction_distance: 0.2, ..IntegrationParameters::default() };
    let mut pipeline = PhysicsPipeline::new();
    let mut islands = IslandManager::new();
    let mut broad = BroadPhaseBvh::new();
    let mut narrow = NarrowPhase::new();
    let mut impulse = ImpulseJointSet::new();
    let mut multibody = MultibodyJointSet::new();
    let mut soft = SoftBodySet::new();
    let mut ccd = CCDSolver::new();
    // Settled: every dynamic body asleep (sensors aside, unless they are all there is).
    let dynamic: Vec<(usize, RigidBodyHandle)> =
        handles.iter().enumerate().filter_map(|(i, h)| Some((i, (*h)?))).filter(|(_, h)| rbs.get(*h).is_some_and(|rb| rb.is_dynamic())).collect();
    let solid: Vec<(usize, RigidBodyHandle)> = dynamic.iter().copied().filter(|(i, _)| bodies.get(*i).is_some_and(|b| !b.settings.sensor)).collect();
    let watched = if solid.is_empty() { dynamic } else { solid };
    let mut lost = vec![false; take];
    let lost_m = (LOST_DISTANCE / PT_PER_M) as f32;
    let mut steps = 0;
    let mut settled = false;
    while steps < max_steps {
        pipeline.step(
            gravity,
            &integration,
            &mut islands,
            &mut broad,
            &mut narrow,
            &mut rbs,
            &mut cols,
            &mut impulse,
            &mut multibody,
            &mut soft,
            &mut ccd,
            &(),
            &(),
        );
        steps += 1;
        // Bodies that fell out of the world stop taking part.
        for (i, h) in handles.iter().enumerate() {
            let Some(rb) = h.and_then(|h| rbs.get_mut(h)) else { continue };
            if rb.is_dynamic() && rb.is_enabled() {
                let t = rb.translation();
                if !(t.x.is_finite() && t.y.is_finite()) || t.x.abs() > lost_m || t.y.abs() > lost_m {
                    rb.set_enabled(false);
                    if let Some(l) = lost.get_mut(i) {
                        *l = true;
                    }
                }
            }
        }
        settled = watched.iter().all(|(i, h)| lost.get(*i).copied().unwrap_or(false) || rbs.get(*h).is_none_or(|rb| rb.is_sleeping()));
        if settled && params.stop_when_settled {
            break;
        }
    }
    let lost_count = lost.iter().filter(|l| **l).count();
    if lost_count > 0 {
        notes.push(format!("{lost_count} bodies fell out of the world and were left where they were"));
    }

    let mut outcomes: Vec<BodyOutcome> = handles
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let is_lost = lost.get(i).copied().unwrap_or(false);
            let moved = (!is_lost)
                .then(|| {
                    let rb = rbs.get((*h)?)?;
                    if !rb.is_dynamic() {
                        return None;
                    }
                    let b = bodies.get(i)?;
                    motion(b.shape.bounds.center(), origin, rb.translation(), rb.rotation().angle())
                })
                .flatten();
            BodyOutcome { moved, lost: is_lost }
        })
        .collect();
    outcomes.extend(bodies.iter().skip(take).map(|_| BodyOutcome { moved: None, lost: false }));
    SimResult { outcomes, steps, settled, notes }
}

/// The document-space motion of a body that started at `start` (its origin, unrotated) and ended
/// at translation `t` (metres from `origin`) and angle `angle`: `None` when it is (nearly) still
/// where it was or isn't finite.
fn motion(start: Point, origin: Point, t: Vector, angle: f32) -> Option<Affine> {
    let end = Point::new(origin.x + f64::from(t.x) * PT_PER_M, origin.y + f64::from(t.y) * PT_PER_M);
    let a = f64::from(angle);
    if !(end.x.is_finite() && end.y.is_finite() && a.is_finite()) {
        return None;
    }
    if (end - start).hypot() < 1e-6 && a.abs() < 1e-9 {
        return None;
    }
    Some(Affine::translate(end.to_vec2()) * Affine::rotate(a) * Affine::translate(-start.to_vec2()))
}

fn add_walls(rbs: &mut RigidBodySet, cols: &mut ColliderSet, r: Rect, origin: Point) {
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|x| x.is_finite()) || r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    let t = WALL_THICKNESS;
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let c = r.center();
    // [centre, half extents]: top, bottom, left, right (tlprocedural's createBoundaryWalls).
    let walls = [
        (Point::new(c.x, r.y0 - t), hw + t, t),
        (Point::new(c.x, r.y1 + t), hw + t, t),
        (Point::new(r.x0 - t, c.y), t, hh + t),
        (Point::new(r.x1 + t, c.y), t, hh + t),
    ];
    for (at, hx, hy) in walls {
        let body = rbs.insert(RigidBodyBuilder::fixed().translation(v(at, origin)));
        let col = ColliderBuilder::cuboid(fin(hx / PT_PER_M).max(MIN_HALF), fin(hy / PT_PER_M).max(MIN_HALF))
            .restitution(WALL_RESTITUTION)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(WALL_FRICTION)
            .friction_combine_rule(CoefficientCombineRule::Min);
        cols.insert_with_parent(col, body, rbs);
    }
}

/// How many pieces `sh` is: a compound's parts, a polyline's segments (each makes its own
/// contacts), else 1.
fn piece_count(sh: &SharedShape) -> usize {
    sh.as_compound().map(|c| c.shapes().len()).or_else(|| sh.as_polyline().map(|p| p.num_segments())).unwrap_or(1).max(1)
}

fn add_body(
    rbs: &mut RigidBodySet,
    cols: &mut ColliderSet,
    b: &BodyInput,
    origin: Point,
    pieces_left: &mut usize,
    boxed: &mut usize,
) -> Option<RigidBodyHandle> {
    let s = &b.settings;
    let center = b.shape.bounds.center();
    let dynamic = s.body_type == BodyType::Dynamic;
    let builder = match &s.body_type {
        BodyType::Dynamic => RigidBodyBuilder::dynamic(),
        BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        // Phase 1's drop-and-settle has no soft bodies: they stay put (the live world moves them).
        BodyType::Static | BodyType::Soft | BodyType::Other(_) => RigidBodyBuilder::fixed(),
    };
    let mut builder = builder.translation(v(center, origin)).gravity_scale(fin(s.gravity_scale)).can_sleep(true);
    if dynamic {
        builder = builder.linear_damping(fin(s.linear_damping)).angular_damping(fin(s.angular_damping)).ccd_enabled(true);
        if s.mass > 0.0 {
            builder = builder.additional_mass(fin(s.mass));
        }
    }
    let mut shapes = shapes_of(&b.shape, s.collider, dynamic, center);
    let count: usize = shapes.iter().map(piece_count).sum();
    if count > *pieces_left {
        shapes = shapes_of(&b.shape, ColliderType::Box, dynamic, center);
        *boxed += 1;
    }
    *pieces_left = pieces_left.saturating_sub(shapes.iter().map(piece_count).sum());
    if shapes.is_empty() {
        return None;
    }
    let solid = shapes.iter().any(|sh| sh.as_polyline().is_none());
    if dynamic && !solid {
        // Only open outlines: no area, so no mass of their own. Give the body its bounds' mass.
        let (w, h) = (b.shape.bounds.width() / PT_PER_M, b.shape.bounds.height() / PT_PER_M);
        let m = (w * h).max(1e-4);
        builder = builder.additional_mass_properties(MassProperties::new(Vector::ZERO, fin(m), fin(m * (w * w + h * h) / 12.0).max(1e-8)));
    }
    let handle = rbs.insert(builder);
    for sh in shapes {
        let col = ColliderBuilder::new(sh)
            .restitution(fin(s.restitution))
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(fin(s.friction))
            .friction_combine_rule(CoefficientCombineRule::Min)
            .sensor(s.sensor);
        cols.insert_with_parent(col, handle, rbs);
    }
    Some(handle)
}

/// Collider shapes for `shape` in body space (metres from `center`).
fn shapes_of(shape: &BodyShape, fidelity: ColliderType, dynamic: bool, center: Point) -> Vec<SharedShape> {
    let local = |pts: &[Point]| -> Vec<Vector> { pts.iter().map(|p| v(*p, center)).collect() };
    let bbox = |r: Rect| {
        let c = v(r.center(), center);
        let shape = SharedShape::cuboid(fin(r.width() / 2.0 / PT_PER_M).max(MIN_HALF), fin(r.height() / 2.0 / PT_PER_M).max(MIN_HALF));
        SharedShape::compound(vec![(Pose::from_translation(c), shape)])
    };
    let hull_or_box = |pts: &[Point], r: Rect| convex(&local(pts)).unwrap_or_else(|| bbox(r));
    match fidelity {
        ColliderType::Box => vec![bbox(shape.bounds)],
        ColliderType::Fast => {
            // tlprocedural: each outline simplified to 32 points, then the hull.
            let pts: Vec<Point> = shape
                .pieces
                .iter()
                .flat_map(|p| match p {
                    Piece::Ring { pts, closed } => simplify(pts, 32, *closed),
                    other => BodyShape { pieces: vec![other.clone()], bounds: shape.bounds, notes: vec![] }.all_points(),
                })
                .collect();
            vec![hull_or_box(&pts, shape.bounds)]
        }
        ColliderType::Auto | ColliderType::Precise => {
            let mut out = Vec::new();
            for p in &shape.pieces {
                match p {
                    Piece::Box(r) => out.push(bbox(*r)),
                    Piece::Ball { center: c, radius } => {
                        let ball = SharedShape::ball(fin(radius / PT_PER_M).max(MIN_HALF));
                        out.push(SharedShape::compound(vec![(Pose::from_translation(v(*c, center)), ball)]));
                    }
                    Piece::Ring { pts, closed: false } => {
                        if let Some(line) = polyline(&local(pts), false) {
                            out.push(line);
                        }
                    }
                    Piece::Ring { pts, closed: true } => {
                        let n = pts.len();
                        let r = ring_bounds(pts).unwrap_or(shape.bounds);
                        if fidelity == ColliderType::Auto && n <= 16 {
                            out.push(hull_or_box(pts, r));
                        } else if fidelity == ColliderType::Auto && n <= 64 {
                            out.push(hull_or_box(&simplify(pts, 32, true), r));
                        } else {
                            let pts = if fidelity == ColliderType::Auto { simplify(pts, 64, true) } else { pts.clone() };
                            out.extend(exact(&pts, r, dynamic, &local, &hull_or_box));
                        }
                    }
                }
            }
            out
        }
    }
}

/// A closed ring exactly: a static body collides with its edges (a cup holds what falls in), a
/// dynamic one is split into convex parts (a trimesh has no volume to push back with).
fn exact(
    pts: &[Point],
    r: Rect,
    dynamic: bool,
    local: &dyn Fn(&[Point]) -> Vec<Vector>,
    hull_or_box: &dyn Fn(&[Point], Rect) -> SharedShape,
) -> Vec<SharedShape> {
    if !dynamic {
        return polyline(&local(pts), true).map_or_else(|| vec![hull_or_box(pts, r)], |l| vec![l]);
    }
    let pts = if pts.len() > MAX_SPLIT_POINTS { simplify(pts, MAX_SPLIT_POINTS, true) } else { pts.to_vec() };
    if signed_area2(&pts).abs() < 1e-9 {
        return vec![hull_or_box(&pts, r)];
    }
    let Some(parts) = convex_parts(&pts) else { return vec![hull_or_box(&pts, r)] };
    let parts: Vec<(Pose, SharedShape)> = parts.iter().filter_map(|part| convex(&local(part))).map(|s| (Pose::IDENTITY, s)).collect();
    if parts.is_empty() { vec![hull_or_box(&pts, r)] } else { vec![SharedShape::compound(parts)] }
}

/// The smallest extent (metres, each axis) and doubled area (m²) a convex collider must have, in
/// the f32 points rapier gets: points distinct in f64 can collapse to one f32 point.
const MIN_EXTENT_M: f64 = 1e-6;
const MIN_AREA2_M: f64 = 1e-10;

/// The convex hull of `pts` (f32, body space) as a convex polygon, or `None` when the points,
/// as f32, don't span an area (then callers use a box). The hull is computed here, on the exact
/// f32 values, and handed to parry's `convex_polyline` (which returns `None` on degenerate input);
/// parry's own `convex_hull` asserts on coincident points, which would abort on wasm.
pub(crate) fn convex(pts: &[Vector]) -> Option<SharedShape> {
    let pts: Vec<Point> = pts.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| Point::new(f64::from(p.x), f64::from(p.y))).collect();
    let hull = convex_hull(&pts);
    if hull.len() < 3 || signed_area2(&hull) < MIN_AREA2_M {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &hull {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    if x1 - x0 < MIN_EXTENT_M || y1 - y0 < MIN_EXTENT_M {
        return None;
    }
    // Exact: the hull's points are the f32 values given.
    SharedShape::convex_polyline(hull.iter().map(|p| Vector::new(p.x as f32, p.y as f32)).collect())
}

/// A polyline through `pts` (closed: back to the first), or `None` with under 2 distinct points.
fn polyline(pts: &[Vector], closed: bool) -> Option<SharedShape> {
    if !pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return None;
    }
    // Repeated f32 points (distinct in f64, collapsed in f32) make zero-length segments: drop them.
    let mut pts: Vec<Vector> = pts.to_vec();
    pts.dedup();
    if closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    let first = *pts.first()?;
    if pts.len() < 2 || pts.iter().all(|p| (*p - first).length() < 1e-7) {
        return None;
    }
    let n = u32::try_from(pts.len()).ok()?;
    let mut idx: Vec<[u32; 2]> = (0..n - 1).map(|i| [i, i + 1]).collect();
    if closed && n >= 3 {
        idx.push([n - 1, 0]);
    }
    Some(SharedShape::polyline(pts, Some(idx)))
}

fn ring_bounds(pts: &[Point]) -> Option<Rect> {
    let first = pts.first()?;
    Some(pts.iter().fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p)))
}
