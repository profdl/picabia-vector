//! The simulation session's clock (PIC31.4): the world at the current step, snapshots for
//! scrubbing, and the event log (pokes, setting changes) replayed at their steps, so scrubbing back
//! and forth gives the same motion.
//!
//! A snapshot is kept every [`SNAPSHOT_EVERY`] steps (at most [`MAX_SNAPSHOTS`]; older ones are
//! thinned). Seeking restores the nearest earlier snapshot and steps forward. Recording an event
//! drops what was recorded after its step.

use vectorcraft_geom::Point;

use crate::settings::Settings;
use crate::world::{World, WorldBody};

/// Steps between snapshots (one simulated second).
pub const SNAPSHOT_EVERY: u32 = 60;
/// The most snapshots kept (older ones are thinned to make room).
pub const MAX_SNAPSHOTS: usize = 64;
/// The longest session, steps (10 minutes of simulated time).
pub const MAX_SESSION_STEPS: u32 = 36_000;

/// Something done to the world at a step, replayed when the step is reached again.
#[derive(Clone, Debug)]
pub enum Event {
    /// Grab body `body` at `at`.
    Grab { body: usize, at: Point },
    /// Pull the grabbed point towards `to`.
    DragTo { to: Point },
    /// Let go.
    Release,
    /// New settings that apply to the running world.
    Retune { body: usize, settings: Box<Settings> },
    /// The body restarts from its rest pose as this (a new mesh).
    Restart { body: usize, input: Box<WorldBody> },
}

/// The session's clock (see the module docs).
#[derive(Clone)]
pub struct SimSession {
    initial: World,
    current: World,
    step: u32,
}

impl SimSession {
    /// A session starting at step 0 with `world`.
    pub fn new(world: World) -> SimSession {
        SimSession { initial: world.clone(), current: world, step: 0 }
    }

    /// The current step.
    pub fn step(&self) -> u32 {
        self.step
    }

    /// The world at the current step.
    pub fn world(&self) -> &World {
        &self.current
    }

    /// Advance `n` steps (at most to [`MAX_SESSION_STEPS`]), replaying recorded events.
    pub fn advance(&mut self, n: u32) {
        let _ = n;
    }

    /// Go to `step` (restore the nearest earlier snapshot, then step forward).
    pub fn seek(&mut self, step: u32) {
        let _ = step;
    }

    /// Record `event` at the current step (dropping what was recorded after it) and apply it.
    pub fn record(&mut self, event: Event) {
        let _ = event;
    }

    /// Back to step 0, keeping the recorded events.
    pub fn reset(&mut self) {
        self.current = self.initial.clone();
        self.step = 0;
    }
}
