//! The live world (PIC31.4): rigid and soft bodies in one rapier world, stepped one step at a time,
//! cloned for snapshots, poked and retuned while it runs.
//!
//! Units as phase 1 ([`crate::PT_PER_M`] points per metre, 60 steps a second). The world is
//! `Clone` (a snapshot); its pipeline is workspace only, so a clone starts a fresh one (PIC31.0:
//! the same bits either way). Soft bodies give the same bits on one machine and build, not across
//! platforms (`docs/picabia/physics.md` › Decisions).

use vectorcraft_geom::{Point, Rect, Vec2};

use crate::mesh::SoftMesh;
use crate::pose::Pose;
use crate::settings::Settings;
use crate::shape::BodyShape;

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

/// The world (see the module docs).
#[derive(Clone, Default)]
pub struct World {
    _private: (),
}

impl World {
    /// A world of `bodies`, with notes on what was left out or simplified. There is one entry per
    /// input body, in order, everywhere bodies are indexed ([`World::poses`], the `body` of
    /// [`World::grab`] and [`World::retune`], [`WorldBody::pins`]); a body left out reads as
    /// `Pose::Rigid(None)` and ignores grabs and retuning.
    pub fn new(bodies: &[WorldBody], params: &WorldParams) -> (World, Vec<String>) {
        let _ = (bodies, params);
        (World::default(), vec!["the live world is not built yet (PIC31.4)".into()])
    }

    /// Advance one step (1/60 s).
    pub fn step(&mut self) {}

    /// Every body's state, in the order given to [`World::new`].
    pub fn poses(&self) -> Vec<Pose> {
        Vec::new()
    }

    /// Every moving body sleeps.
    pub fn settled(&self) -> bool {
        true
    }

    /// Grab body `body` at `at` (document space): a soft body by its nearest particle, a rigid one
    /// by that point. `false` when it can't be grabbed (static, unknown).
    pub fn grab(&mut self, body: usize, at: Point) -> bool {
        let _ = (body, at);
        false
    }

    /// Pull the grabbed point towards `to` (a spring, applied each step until released).
    pub fn drag_to(&mut self, to: Point) {
        let _ = to;
    }

    /// Let go: the body keeps its speed (a throw).
    pub fn release(&mut self) {}

    /// New settings for body `body` that don't need a new mesh (material, gravity, bounce,
    /// friction, damping), from the next step.
    pub fn retune(&mut self, body: usize, settings: &Settings) {
        let _ = (body, settings);
    }

    /// Put body `body` back at its rest pose as `new` (a new mesh, a new kind of body).
    pub fn restart_body(&mut self, body: usize, new: &WorldBody) {
        let _ = (body, new);
    }
}
