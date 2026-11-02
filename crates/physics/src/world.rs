//! The live world (PIC31.4): rigid and soft bodies in one rapier world, stepped one step at a time,
//! cloned for snapshots, poked and retuned while it runs.
//!
//! Units as phase 1 ([`crate::PT_PER_M`] points per metre, 60 steps a second). Rigid bodies and the
//! walls are built exactly as phase 1 builds them (`sim.rs`). Soft bodies come from their rest
//! meshes: cloth as a `polyline` without shape matching, jelly and rubber as cells with area
//! preservation and (with shape memory) shape matching; their mass is 1 kg/m² of area plus the
//! extra mass, as rigid bodies'. PIC31.0's cost findings are applied: no extra PGS iterations and
//! no impact substeps for soft bodies, no self-contact unless asked.
//!
//! The world is `Clone` (a snapshot); its pipeline is workspace only, so a clone starts a fresh one
//! (PIC31.0: the same bits either way). Soft bodies give the same bits on one machine and build, not
//! across platforms (`docs/picabia/physics.md` › Decisions).

use rapier2d::prelude::*;
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::PT_PER_M;
use crate::mesh::{MeshKind, SoftMesh};
use crate::pose::Pose;
use crate::settings::{BodyType, Settings};
use crate::shape::BodyShape;
use crate::sim::{self, BodyInput, HZ, LOST_DISTANCE, MAX_GRAVITY, MAX_PIECES_PER_RUN};

/// The pull of a poke: the spring's natural frequency, Hz (critically damped).
pub const POKE_HZ: f32 = 4.0;

/// One body to put in the world.
#[derive(Clone, Debug)]
pub struct WorldBody {
    pub settings: Settings,
    pub shape: BodyShape,
    /// The rest mesh, for a soft body.
    pub mesh: Option<SoftMesh>,
    /// For each of the soft body's pins: the particle it holds and the index (in the world's
    /// bodies) of the body it is attached to, `None` when pinned in place.
    pub pins: Vec<(u32, Option<usize>)>,
}

/// The world's parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldParams {
    /// Points per second², document axes.
    pub gravity: Vec2,
    /// Walls around this rectangle (the artboard), or none.
    pub walls: Option<Rect>,
}

/// A clone of the world starts with a fresh pipeline (workspace only).
struct Pipe(PhysicsPipeline);

impl Clone for Pipe {
    fn clone(&self) -> Self {
        Pipe(PhysicsPipeline::new())
    }
}

/// Where a body lives in the rapier sets.
#[derive(Clone, Debug)]
enum Slot {
    /// Not in the world (it had no outline, or fell out).
    None,
    Rigid {
        handle: RigidBodyHandle,
        start: Point,
    },
    Soft {
        handle: SoftBodyHandle,
    },
}

/// What a poke holds.
#[derive(Clone, Debug)]
struct Grab {
    body: usize,
    /// Rigid: the point in the body's frame; soft: unused.
    local: Vector,
    /// Soft: the particle.
    particle: Option<usize>,
    target: Vector,
}

/// The world (see the module docs).
#[derive(Clone)]
pub struct World {
    rbs: RigidBodySet,
    cols: ColliderSet,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    impulse: ImpulseJointSet,
    multibody: MultibodyJointSet,
    soft: SoftBodySet,
    ccd: CCDSolver,
    pipe: Pipe,
    ip: IntegrationParameters,
    gravity: Vector,
    origin: Point,
    slots: Vec<Slot>,
    grab: Option<Grab>,
    pieces_left: usize,
}

fn fin(x: f64) -> f32 {
    if x.is_finite() { x as f32 } else { 0.0 }
}

impl World {
    /// A world of `bodies`, with notes on what was left out or simplified. There is one entry per
    /// input body, in order, everywhere bodies are indexed ([`World::poses`], the `body` of
    /// [`World::grab`] and [`World::retune`], [`WorldBody::pins`]); a body left out reads as
    /// `Pose::Rigid(None)` and ignores grabs and retuning.
    pub fn new(bodies: &[WorldBody], params: &WorldParams) -> (World, Vec<String>) {
        let mut notes = Vec::new();
        let g = |x: f64| if x.is_finite() { x.clamp(-MAX_GRAVITY, MAX_GRAVITY) } else { 0.0 };
        let origin = params
            .walls
            .filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|x| x.is_finite()))
            .map(|r| r.center())
            .or_else(|| bodies.iter().map(|b| b.shape.bounds).reduce(|a, b| a.union(b)).map(|r| r.center()))
            .unwrap_or(Point::ZERO);
        // As phase 1: a 20 pt contact margin. PIC31.0: soft bodies run without impact substeps.
        let mut ip = IntegrationParameters { dt: (1.0 / HZ) as f32, normalized_prediction_distance: 0.2, ..IntegrationParameters::default() };
        ip.soft_bodies.max_extra_substeps = 0;
        let mut w = World {
            rbs: RigidBodySet::new(),
            cols: ColliderSet::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            impulse: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            pipe: Pipe(PhysicsPipeline::new()),
            ip,
            gravity: Vector::new(fin(g(params.gravity.x) / PT_PER_M), fin(g(params.gravity.y) / PT_PER_M)),
            origin,
            slots: Vec::with_capacity(bodies.len()),
            grab: None,
            pieces_left: MAX_PIECES_PER_RUN,
        };
        let mut boxed = 0usize;
        // Rigid bodies first, so soft pins can attach to any of them.
        let mut slots = vec![Slot::None; bodies.len()];
        for (i, b) in bodies.iter().enumerate() {
            if b.mesh.is_none()
                && let Some(s) = slots.get_mut(i)
            {
                *s = w.add_rigid(b, &mut boxed);
            }
        }
        w.slots = slots;
        for (i, b) in bodies.iter().enumerate() {
            if b.mesh.is_some() {
                let slot = w.add_soft(b, &mut notes);
                if let Some(s) = w.slots.get_mut(i) {
                    *s = slot;
                }
            }
        }
        if boxed > 0 {
            notes.push(format!("over {MAX_PIECES_PER_RUN} collider pieces in all: {boxed} bodies collide as their bounding boxes"));
        }
        if let Some(r) = params.walls {
            sim::add_walls(&mut w.rbs, &mut w.cols, r, origin);
        }
        (w, notes)
    }

    fn add_rigid(&mut self, b: &WorldBody, boxed: &mut usize) -> Slot {
        let input = BodyInput { settings: b.settings.clone(), shape: b.shape.clone() };
        match sim::add_body(&mut self.rbs, &mut self.cols, &input, self.origin, &mut self.pieces_left, boxed) {
            Some(handle) => Slot::Rigid { handle, start: b.shape.bounds.center() },
            None => Slot::None,
        }
    }

    fn to_m(&self, p: Point) -> Vector {
        Vector::new(fin((p.x - self.origin.x) / PT_PER_M), fin((p.y - self.origin.y) / PT_PER_M))
    }

    fn to_pt(&self, v: Vector) -> Point {
        Point::new(self.origin.x + f64::from(v.x) * PT_PER_M, self.origin.y + f64::from(v.y) * PT_PER_M)
    }

    fn add_soft(&mut self, b: &WorldBody, notes: &mut Vec<String>) -> Slot {
        let (Some(mesh), Some(soft)) = (&b.mesh, b.settings.soft_body()) else { return Slot::None };
        let s = &b.settings;
        let p = soft.fields().params();
        let pts: Vec<Vector> = mesh.rest.iter().map(|q| self.to_m(*q)).collect();
        let radius = fin(mesh.radius / PT_PER_M).max(1e-4);
        let builder = match &mesh.kind {
            MeshKind::Strip(_) => match SoftBodyBuilder::polyline(pts, None) {
                Some(bl) => bl.shape_matching(false).self_contacts(p.self_collide),
                None => return Slot::None,
            },
            MeshKind::Cells => {
                if mesh.cells.is_empty() {
                    return Slot::None;
                }
                SoftBodyBuilder::new(pts)
                    .cells(mesh.cells.clone())
                    .volume_preservation(true)
                    .shape_matching(p.shape_hz > 0.0)
                    .self_contacts(p.self_collide)
            }
        };
        // 1 kg/m² of area, as rigid bodies, plus the extra mass.
        let area = match &mesh.kind {
            MeshKind::Strip(st) => st.length * st.thickness,
            MeshKind::Cells => b.shape.bounds.width() * b.shape.bounds.height() * 0.5,
        } / (PT_PER_M * PT_PER_M);
        let mass = (area + s.mass).max(1e-3);
        let pinned: Vec<u32> = b.pins.iter().filter(|(_, to)| to.is_none()).map(|(i, _)| *i).collect();
        let template = ColliderBuilder::ball(0.05)
            .density(0.0)
            .friction(fin(s.friction))
            .friction_combine_rule(CoefficientCombineRule::Min)
            .restitution(fin(s.restitution))
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .sensor(s.sensor);
        let builder = builder
            .particle_radius(radius)
            .material(material(&p))
            .mass(fin(mass))
            .pinned_particles(pinned)
            .surface_collider(template)
            .gravity_scale(fin(s.gravity_scale))
            .linear_damping(fin(s.linear_damping))
            .additional_pgs_iterations(0);
        let handle = self.soft.insert(builder, &mut self.rbs, &mut self.cols);
        // Attached pins: held to their rigid body.
        for (i, to) in &b.pins {
            let Some(to) = to else { continue };
            match self.slots.get(*to) {
                Some(Slot::Rigid { handle: rb, .. }) => {
                    if let Some(sb) = self.soft.get_mut(handle)
                        && (*i as usize) < sb.num_particles()
                    {
                        sb.attach_particle(*i as usize, *rb, &self.rbs);
                    }
                }
                _ => notes.push("a pin is attached to an object that isn't a rigid body in this simulation: pinned in place".into()),
            }
            // A pin whose body is missing still holds its particle in place.
            if !matches!(self.slots.get(*to), Some(Slot::Rigid { .. }))
                && let Some(sb) = self.soft.get_mut(handle)
                && (*i as usize) < sb.num_particles()
            {
                sb.set_particle_pinned(*i as usize, true);
            }
        }
        Slot::Soft { handle }
    }

    /// Advance one step (1/60 s).
    pub fn step(&mut self) {
        self.pull();
        self.pipe.0.step(
            self.gravity,
            &self.ip,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.rbs,
            &mut self.cols,
            &mut self.impulse,
            &mut self.multibody,
            &mut self.soft,
            &mut self.ccd,
            &(),
            &(),
        );
        // Rigid bodies that fell out of the world stop taking part (as phase 1).
        let lost_m = (LOST_DISTANCE / PT_PER_M) as f32;
        for slot in &mut self.slots {
            if let Slot::Rigid { handle, .. } = slot
                && let Some(rb) = self.rbs.get_mut(*handle)
                && rb.is_dynamic()
                && rb.is_enabled()
            {
                let t = rb.translation();
                if !(t.x.is_finite() && t.y.is_finite()) || t.x.abs() > lost_m || t.y.abs() > lost_m {
                    rb.set_enabled(false);
                }
            }
        }
    }

    /// The poke's spring force, set before each step (cleared when nothing is held).
    fn pull(&mut self) {
        let Some(g) = self.grab.clone() else { return };
        let w = std::f32::consts::TAU * POKE_HZ;
        let (k, c) = (w * w, 2.0 * w);
        match self.slots.get(g.body) {
            Some(Slot::Rigid { handle, .. }) => {
                if let Some(rb) = self.rbs.get_mut(*handle) {
                    let at = rb.position() * g.local;
                    let vel = rb.velocity_at_point(at);
                    let f = ((g.target - at) * k - vel * c) * rb.mass();
                    rb.reset_forces(true);
                    if f.x.is_finite() && f.y.is_finite() {
                        rb.add_force_at_point(f, at, true);
                    }
                }
            }
            Some(Slot::Soft { handle }) => {
                if let (Some(sb), Some(i)) = (self.soft.get_mut(*handle), g.particle)
                    && i < sb.num_particles()
                {
                    // The pull of the whole body's mass on one particle stretches it apart: a
                    // particle pulls with the geometric mean of its own mass and the body's.
                    let m = (sb.mass() * sb.mass() / sb.num_particles().max(1) as f32).sqrt();
                    let f = ((g.target - sb.particle_position(i)) * k - sb.particle_velocity(i) * c) * m;
                    sb.reset_forces(true);
                    if f.x.is_finite() && f.y.is_finite() {
                        sb.add_particle_force(i, f, true);
                    }
                }
            }
            _ => {}
        }
    }

    /// Every body's state, in the order given to [`World::new`].
    pub fn poses(&self) -> Vec<Pose> {
        self.slots
            .iter()
            .map(|slot| match slot {
                Slot::None => Pose::Rigid(None),
                Slot::Rigid { handle, start } => Pose::Rigid(
                    self.rbs
                        .get(*handle)
                        .filter(|rb| rb.is_dynamic() && rb.is_enabled())
                        .and_then(|rb| sim::motion(*start, self.origin, rb.translation(), rb.rotation().angle())),
                ),
                Slot::Soft { handle } => {
                    Pose::Soft(self.soft.get(*handle).map_or_else(Vec::new, |sb| sb.particle_positions().map(|v| self.to_pt(v)).collect()))
                }
            })
            .collect()
    }

    /// Every moving body sleeps (bodies that fell out aside).
    pub fn settled(&self) -> bool {
        self.slots.iter().all(|slot| match slot {
            Slot::None => true,
            Slot::Rigid { handle, .. } => self.rbs.get(*handle).is_none_or(|rb| !rb.is_dynamic() || !rb.is_enabled() || rb.is_sleeping()),
            Slot::Soft { handle } => self.soft.get(*handle).is_none_or(|sb| sb.is_sleeping() || !sb.is_enabled()),
        })
    }

    /// Whether body `body` can be grabbed (a dynamic rigid body or a soft body in the world).
    pub fn movable(&self, body: usize) -> bool {
        match self.slots.get(body) {
            Some(Slot::Rigid { handle, .. }) => self.rbs.get(*handle).is_some_and(|rb| rb.is_dynamic() && rb.is_enabled()),
            Some(Slot::Soft { handle }) => self.soft.contains(*handle),
            _ => false,
        }
    }

    /// Grab body `body` at `at` (document space): a soft body by its nearest particle, a rigid one
    /// by that point. `false` when it can't be grabbed (static, unknown).
    pub fn grab(&mut self, body: usize, at: Point) -> bool {
        if !self.movable(body) || !(at.x.is_finite() && at.y.is_finite()) {
            return false;
        }
        let target = self.to_m(at);
        match self.slots.get(body) {
            Some(Slot::Rigid { handle, .. }) => {
                let Some(rb) = self.rbs.get(*handle) else { return false };
                let local = rb.position().inverse_transform_point(target);
                self.grab = Some(Grab { body, local, particle: None, target });
            }
            Some(Slot::Soft { handle }) => {
                let Some(sb) = self.soft.get(*handle) else { return false };
                let mut best: Option<(usize, f32)> = None;
                for (i, q) in sb.particle_positions().enumerate() {
                    let d = (q - target).length_squared();
                    if best.is_none_or(|(_, b)| d < b) {
                        best = Some((i, d));
                    }
                }
                let Some((i, _)) = best else { return false };
                self.grab = Some(Grab { body, local: Vector::ZERO, particle: Some(i), target });
            }
            _ => return false,
        }
        true
    }

    /// Pull the grabbed point towards `to` (a spring, applied each step until released).
    pub fn drag_to(&mut self, to: Point) {
        if !(to.x.is_finite() && to.y.is_finite()) {
            return;
        }
        let t = self.to_m(to);
        if let Some(g) = &mut self.grab {
            g.target = t;
        }
    }

    /// Let go: the body keeps its speed (a throw).
    pub fn release(&mut self) {
        let Some(g) = self.grab.take() else { return };
        match self.slots.get(g.body) {
            Some(Slot::Rigid { handle, .. }) => {
                if let Some(rb) = self.rbs.get_mut(*handle) {
                    rb.reset_forces(true);
                }
            }
            Some(Slot::Soft { handle }) => {
                if let Some(sb) = self.soft.get_mut(*handle) {
                    sb.reset_forces(true);
                }
            }
            _ => {}
        }
    }

    /// The body held by a poke, if any.
    pub fn grabbed(&self) -> Option<usize> {
        self.grab.as_ref().map(|g| g.body)
    }

    /// New settings for body `body` that don't need a new mesh (material, bounce, friction,
    /// damping; a rigid body's gravity too), from the next step.
    pub fn retune(&mut self, body: usize, s: &Settings) {
        match self.slots.get(body) {
            Some(Slot::Rigid { handle, .. }) => {
                let colliders: Vec<ColliderHandle> = self.rbs.get(*handle).map(|rb| rb.colliders().to_vec()).unwrap_or_default();
                if let Some(rb) = self.rbs.get_mut(*handle) {
                    rb.set_gravity_scale(fin(s.gravity_scale), true);
                    if rb.is_dynamic() {
                        rb.set_linear_damping(fin(s.linear_damping));
                        rb.set_angular_damping(fin(s.angular_damping));
                    }
                    rb.wake_up(true);
                }
                for c in colliders {
                    if let Some(col) = self.cols.get_mut(c) {
                        col.set_restitution(fin(s.restitution));
                        col.set_friction(fin(s.friction));
                        col.set_sensor(s.sensor);
                    }
                }
            }
            Some(Slot::Soft { handle }) => {
                let Some(soft) = s.soft_body() else { return };
                let p = soft.fields().params();
                let mut colliders = Vec::new();
                if let Some(sb) = self.soft.get_mut(*handle) {
                    sb.set_material(material(&p));
                    colliders.extend(sb.meshes().map(|m| m.collider()));
                    sb.wake_up();
                }
                for c in colliders {
                    if let Some(col) = self.cols.get_mut(c) {
                        col.set_restitution(fin(s.restitution));
                        col.set_friction(fin(s.friction));
                        col.set_sensor(s.sensor);
                    }
                }
            }
            _ => {}
        }
    }

    /// Put body `body` back at its rest pose as `new` (a new mesh, a new kind of body).
    pub fn restart_body(&mut self, body: usize, new: &WorldBody) {
        if self.grab.as_ref().is_some_and(|g| g.body == body) {
            self.grab = None;
        }
        match self.slots.get(body).cloned() {
            Some(Slot::Rigid { handle, .. }) => {
                self.rbs.remove(handle, &mut self.islands, &mut self.cols, &mut self.impulse, &mut self.multibody, &mut self.soft, true);
            }
            Some(Slot::Soft { handle }) => {
                self.soft.remove(handle, &mut self.islands, &mut self.rbs, &mut self.cols, &mut self.impulse, &mut self.multibody);
            }
            _ => {}
        }
        let mut boxed = 0;
        let mut notes = Vec::new();
        let slot = if new.mesh.is_some() { self.add_soft(new, &mut notes) } else { self.add_rigid(new, &mut boxed) };
        if let Some(s) = self.slots.get_mut(body) {
            *s = slot;
        }
    }

    /// The number of bodies.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// The soft body material for `p` (PIC31.1's mapping of the 0–1 fields).
fn material(p: &crate::soft::SoftParams) -> SoftBodyMaterial {
    let z = p.damping_ratio;
    SoftBodyMaterial {
        edge_softness: SpringCoefficients::new(p.edge_hz, z),
        bend_softness: SpringCoefficients::new(p.bend_hz, z),
        volume_softness: SpringCoefficients::new(p.volume_hz, z),
        shape_matching_softness: SpringCoefficients::new(p.shape_hz.max(0.1), z),
        ..SoftBodyMaterial::default()
    }
}

/// Body types the world moves.
pub fn moves(t: &BodyType) -> bool {
    matches!(t, BodyType::Dynamic | BodyType::Soft)
}
