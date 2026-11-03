//! The simulation session's clock (PIC31.4): the world at the current step, snapshots for
//! scrubbing, and the event log (pokes, setting changes) replayed at their steps, so scrubbing back
//! and forth gives the same motion.
//!
//! A snapshot is kept every [`SNAPSHOT_EVERY`] steps (at most [`MAX_SNAPSHOTS`]; when full, every
//! other one is dropped and they are kept twice as far apart). Seeking restores the nearest earlier
//! snapshot and steps forward. Recording an event drops what was recorded after its step (and the
//! snapshots after it), so playing or poking after scrubbing back starts a new future.

use vectorcraft_geom::Point;

use crate::settings::Settings;
use crate::world::{World, WorldBody};

/// Steps between snapshots at first (one simulated second).
pub const SNAPSHOT_EVERY: u32 = 60;
/// The most snapshots kept (older ones are thinned to make room).
pub const MAX_SNAPSHOTS: usize = 64;
/// The longest session, steps (10 minutes of simulated time).
pub const MAX_SESSION_STEPS: u32 = 36_000;
/// The most events recorded (a drag records a pull a frame; consecutive pulls in one step keep the
/// last). Past it, new events are refused.
pub const MAX_EVENTS: usize = 20_000;

/// Something done to the world at a step, replayed when the step is reached again. Events at the
/// same step apply in the order they were recorded.
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

impl Event {
    fn apply(&self, w: &mut World) {
        match self {
            Event::Grab { body, at } => {
                w.grab(*body, *at);
            }
            Event::DragTo { to } => w.drag_to(*to),
            Event::Release => w.release(),
            Event::Retune { body, settings } => w.retune(*body, settings),
            Event::Restart { body, input } => w.restart_body(*body, input),
        }
    }
}

/// The session's clock (see the module docs).
#[derive(Clone)]
pub struct SimSession {
    initial: World,
    current: World,
    step: u32,
    /// Events, in step order (several may share a step: they apply in order).
    events: Vec<(u32, Event)>,
    /// Snapshots `(step, world)`, in step order; the world as it was *before* that step's events.
    snapshots: Vec<(u32, World)>,
    every: u32,
}

impl SimSession {
    /// A session starting at step 0 with `world`.
    pub fn new(world: World) -> SimSession {
        SimSession { initial: world.clone(), current: world, step: 0, events: Vec::new(), snapshots: Vec::new(), every: SNAPSHOT_EVERY }
    }

    /// The current step.
    pub fn step(&self) -> u32 {
        self.step
    }

    /// The world at the current step.
    pub fn world(&self) -> &World {
        &self.current
    }

    /// The body a poke holds once this step's recorded events apply (they apply when the next step
    /// is taken), so a grab and a pull can be recorded at the same step.
    pub fn held(&self) -> Option<usize> {
        let mut held = self.current.grabbed();
        for (_, e) in self.events.iter().filter(|(s, _)| *s == self.step) {
            match e {
                Event::Grab { body, .. } => held = Some(*body),
                Event::Release => held = None,
                _ => {}
            }
        }
        held
    }

    /// The recorded events.
    pub fn events(&self) -> &[(u32, Event)] {
        &self.events
    }

    /// The steps snapshots are kept at.
    pub fn snapshot_steps(&self) -> Vec<u32> {
        self.snapshots.iter().map(|(s, _)| *s).collect()
    }

    /// Advance `n` steps (at most to [`MAX_SESSION_STEPS`]), replaying recorded events. Returns
    /// the steps taken.
    pub fn advance(&mut self, n: u32) -> u32 {
        let n = n.min(MAX_SESSION_STEPS.saturating_sub(self.step));
        for _ in 0..n {
            self.one();
        }
        n
    }

    /// Advance until every body rests, or `max` steps. Returns the steps taken.
    pub fn to_rest(&mut self, max: u32) -> u32 {
        let max = max.min(MAX_SESSION_STEPS.saturating_sub(self.step));
        let mut done = 0;
        while done < max {
            self.one();
            done += 1;
            if self.current.settled() && self.current.grabbed().is_none() && self.events.iter().all(|(s, _)| *s < self.step) {
                break;
            }
        }
        done
    }

    /// One step: keep a snapshot when due, apply this step's events, step.
    fn one(&mut self) {
        if self.step > 0 && self.step.is_multiple_of(self.every) && self.snapshots.last().is_none_or(|(s, _)| *s < self.step) {
            self.snapshots.push((self.step, self.current.clone()));
            if self.snapshots.len() > MAX_SNAPSHOTS {
                // Thin: keep every other one, twice as far apart from now on.
                self.every = self.every.saturating_mul(2);
                let every = self.every;
                self.snapshots.retain(|(s, _)| s % every == 0);
            }
        }
        let step = self.step;
        for (_, e) in self.events.iter().filter(|(s, _)| *s == step) {
            e.apply(&mut self.current);
        }
        self.current.step();
        self.step += 1;
    }

    /// Go to `step` (restore the nearest earlier snapshot, then step forward).
    pub fn seek(&mut self, step: u32) {
        let step = step.min(MAX_SESSION_STEPS);
        if step == self.step {
            return;
        }
        if step < self.step {
            match self.snapshots.iter().rev().find(|(s, _)| *s <= step) {
                Some((s, w)) => {
                    self.current = w.clone();
                    self.step = *s;
                }
                None => {
                    self.current = self.initial.clone();
                    self.step = 0;
                }
            }
        }
        while self.step < step {
            self.one();
        }
    }

    /// Record `event` at the current step (dropping what was recorded after it, and the snapshots
    /// after it); `false` when [`MAX_EVENTS`] are recorded already. It applies when the next step is
    /// taken, after that step's snapshot, so a snapshot is always the world before its step's events
    /// and replaying applies each event once.
    pub fn record(&mut self, event: Event) -> bool {
        let now = self.step;
        self.events.retain(|(s, _)| *s <= now);
        self.snapshots.retain(|(s, _)| *s <= now);
        // Consecutive pulls in one step: the last one is all that counts.
        if matches!(event, Event::DragTo { .. }) && matches!(self.events.last(), Some((s, Event::DragTo { .. })) if *s == now) {
            self.events.pop();
        }
        if self.events.len() >= MAX_EVENTS {
            return false;
        }
        self.events.push((now, event));
        true
    }

    /// Back to step 0, keeping the recorded events.
    pub fn reset(&mut self) {
        self.current = self.initial.clone();
        self.step = 0;
    }

    /// Change the world at step 0 (a body restarted for good): the session starts again from it,
    /// events and snapshots dropped.
    pub fn restart(&mut self, world: World) {
        *self = SimSession::new(world);
    }
}
