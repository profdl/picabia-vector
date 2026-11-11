//! PICABIA: the live physics session (PIC31.4): rigid and soft bodies in one world, played,
//! paused, scrubbed and poked, shown as clones, and baked into the art by Apply.
//!
//! The session is the engine's working state (`Session::picabia.physics`), never the document's:
//! nothing in the document changes until Apply (one undo step, "Apply Physics"), so saving while a
//! session is open saves the art at rest. It belongs to the document it was opened in; when that
//! document changes (an edit, an undo) the session starts again from the new rest pose, with a note.
//! Setting changes made through `physics.set` while it runs are applied to the running world
//! instead (PIC31.7).
//!
//! The clock: 60 steps a simulated second. The app calls [`tick`] every frame while playing; it
//! takes at most [`MAX_STEPS_PER_TICK`] steps a frame, so a slow scene plays slower than real time
//! but stays exact. Scripts and agents use the commands (`physics.seek`, `physics.toRest`).

use std::fmt;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_physics::mesh::SoftMesh;
use vectorcraft_physics::pose::Pose;
use vectorcraft_physics::session::{Event, MAX_EVENTS, MAX_SESSION_STEPS, SimSession};
use vectorcraft_physics::sim;
use vectorcraft_physics::world::{World, WorldBody, WorldParams};

use super::apply::{Baking, bake};
use super::{Gathered, gather};
use crate::cmd::{CommandSpec, bad, check_numbers, cmd, has_doc, number_or, switch_or};
use crate::{EngineError, Result, Session};

/// The most steps one [`tick`] takes (a slow scene plays slower than real time).
pub const MAX_STEPS_PER_TICK: u32 = 4;
/// Steps Simulate to Rest runs at most when not told (50 simulated seconds, as phase 1).
pub const DEFAULT_TO_REST_STEPS: u32 = sim::DEFAULT_MAX_STEPS;

/// An open session.
pub(crate) struct Live {
    /// The document it belongs to.
    pub(crate) doc_uid: u64,
    /// The document at rest: the session restarts when the document is no longer this one.
    pub(crate) base: Arc<Document>,
    /// The bodies' objects, in the world's order.
    pub(crate) ids: Vec<NodeId>,
    /// What each body was built from (for restarts).
    pub(crate) bodies: Vec<WorldBody>,
    pub(crate) params: WorldParams,
    pub(crate) sim: SimSession,
    pub(crate) playing: bool,
    /// Wall time not yet stepped, seconds.
    pub(crate) owed: f64,
    pub(crate) notes: Vec<String>,
    pub(crate) skipped: Vec<Value>,
    pub(crate) warnings: Vec<Value>,
    /// The artboard the walls stand on.
    pub(crate) artboard: Option<usize>,
    /// The furthest step reached (the scrubber's end).
    pub(crate) furthest: u32,
    /// Pattern fills move with rigid bodies' clones (General › Transform Pattern Tiles), as Apply
    /// moves them.
    pub(crate) patterns: bool,
    /// Objects without settings that take part (they were selected when it opened).
    pub(crate) defaulted: std::collections::HashSet<NodeId>,
    /// The document the canvas draws at the current step ([`display_doc`]).
    pub(crate) shown: Option<Arc<Document>>,
}

impl fmt::Debug for Live {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Live")
            .field("doc", &self.doc_uid)
            .field("bodies", &self.ids.len())
            .field("step", &self.sim.step())
            .field("playing", &self.playing)
            .finish()
    }
}

impl Live {
    /// The world's poses with their objects and meshes, for Apply and the clones. A body within
    /// [`STILL`] of its rest pose hasn't moved (`Pose::Rigid(None)`): the world is f32, so a body
    /// at rest comes back a hair off, which mustn't turn a live shape into a path.
    pub(crate) fn poses(&self) -> Vec<(NodeId, Pose, Option<&SoftMesh>)> {
        let world = self.sim.world();
        // Each pose with the mesh the world has for it now (a restart changes it; scrubbing back
        // before the restart brings the old one back).
        self.ids
            .iter()
            .zip(world.poses())
            .enumerate()
            .filter_map(|(i, (id, pose))| Some(((id, pose), world.body(i)?)))
            .map(|((id, pose), b)| {
                let still = match (&pose, &b.mesh) {
                    (Pose::Soft(p), Some(m)) => p.len() == m.rest.len() && p.iter().zip(&m.rest).all(|(a, r)| (*a - *r).hypot() < STILL),
                    (Pose::Rigid(Some(xf)), _) => {
                        let r = b.shape.bounds;
                        [r.origin(), vectorcraft_geom::Point::new(r.x1, r.y1)].iter().all(|c| (*xf * *c - *c).hypot() < STILL)
                    }
                    _ => false,
                };
                (*id, if still { Pose::Rigid(None) } else { pose }, b.mesh.as_ref())
            })
            .collect()
    }

    /// The body index of object `id`.
    pub(crate) fn body_of(&self, id: NodeId) -> Option<usize> {
        self.ids.iter().position(|i| *i == id)
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "physics.session.open",
            "Start Simulation",
            [],
            None,
            "{gravityX?, gravityY? (pt/s², default 0, 981), walls? (default true), artboard? (index; default: the one under the moving bodies)} open a live session in the active document: every visible object with physics settings turned on takes part (selected objects without settings as dynamic bodies), rigid and soft in one world, at step 0 and paused; nothing in the document changes until physics.apply. Replaces a session already open → {bodies, soft, steps: 0, skipped: [{id, reason}], warnings: [{id, message}], notes, artboard}",
            has_doc,
            open
        ),
        cmd!(
            "physics.session.close",
            "Close Simulation",
            [],
            None,
            "{} close the session without changing the art (as physics.cancel)",
            has_session,
            close
        ),
        cmd!(
            query "physics.session.info",
            "Simulation Info",
            [],
            None,
            "{} → {open, step?, seconds?, playing?, settled?, bodies?: [{id, kind}], events?, notes?, skipped?, warnings?, artboard?}",
            has_doc,
            info
        ),
        cmd!(
            "physics.play",
            "Play Simulation",
            [],
            None,
            "{gravityX?, gravityY?, walls?, artboard? (as physics.session.open, when it opens one)} play the session (the app steps it at 60 steps a second); opens one when none is",
            has_doc,
            play
        ),
        cmd!("physics.pause", "Pause Simulation", [], None, "{} pause the session", has_session, pause),
        cmd!(
            "physics.reset",
            "Reset Simulation",
            [],
            None,
            "{} back to step 0 and pause (recorded pokes and setting changes are kept)",
            has_session,
            reset
        ),
        cmd!(
            query "physics.seek",
            "Seek Simulation",
            [],
            None,
            "{step (0-36000: the moment, 60 steps a second)} go to that step: back from the nearest snapshot, forward by stepping; recorded pokes and setting changes replay",
            has_session,
            seek
        ),
        cmd!(
            "physics.toRest",
            "Simulate to Rest",
            [],
            None,
            "{steps? (most steps to run, 1-36000; default 3000), gravityX?, gravityY?, walls?, artboard? (as physics.session.open, when it opens one)} run until every body rests (or the cap), opening a session when none is, then pause: the result shows until Apply or Cancel → {step, seconds, settled}",
            has_doc,
            to_rest
        ),
        cmd!(
            query "physics.poke",
            "Poke",
            [],
            None,
            "{id: a moving body's object, point: {x, y} where it is grabbed (document space; a soft body by its nearest particle), to?: {x, y} where it is pulled (a spring, each step until released), release?: let go (it keeps its speed)} record a poke at the current step: scrubbing back and forth replays it; what was recorded after this step is dropped",
            has_session,
            poke
        ),
        cmd!(
            "physics.apply",
            "Apply Physics",
            [],
            None,
            "{patterns?} bake the shown pose into every body's object (rigid: moved and turned; soft: bent) as one undo step, and close the session → {changed: [ids]}",
            has_session,
            apply
        ),
        cmd!("physics.cancel", "Cancel Simulation", [], None, "{} close the session; the art is left as it was", has_session, close),
    ]
}

fn has_session(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if live(s).is_some() { Ok(()) } else { Err("no simulation is open (Object ▸ Physics ▸ Start Simulation)".into()) }
}

/// The open session, when it belongs to the active document.
pub(crate) fn live(s: &Session) -> Option<&Live> {
    let uid = s.active()?.uid;
    s.picabia.physics.as_deref().filter(|l| l.doc_uid == uid)
}

fn live_mut(s: &mut Session) -> Option<&mut Live> {
    let uid = s.active()?.uid;
    s.picabia.physics.as_deref_mut().filter(|l| l.doc_uid == uid)
}

/// Build a session from the active document (`extra`: objects without settings to take part;
/// `None`: the selection).
fn build(s: &mut Session, p: &Value, cmd: &str, extra: Option<&std::collections::HashSet<NodeId>>) -> Result<Live> {
    let g: Gathered = gather(s, p, cmd, extra)?;
    if !g.bodies.iter().any(|b| vectorcraft_physics::world::moves(&b.settings.body_type)) {
        return Err(EngineError::Disabled(
            cmd.into(),
            "nothing moves: select objects, or give objects physics settings (Object ▸ Physics ▸ Add Physics)".into(),
        ));
    }
    let (world, notes) = World::new(&g.bodies, &g.params);
    let st = s.doc()?;
    Ok(Live {
        doc_uid: st.uid,
        base: st.doc.clone(),
        ids: g.ids,
        bodies: g.bodies,
        params: g.params,
        sim: SimSession::new(world),
        playing: false,
        owed: 0.0,
        notes,
        patterns: s.prefs.transform_pattern_tiles,
        defaulted: g.defaulted.iter().copied().collect(),
        skipped: g.skipped,
        warnings: g.warnings,
        artboard: g.artboard,
        furthest: 0,
        shown: None,
    })
}

/// The session's view of its document: started again when the document changed under it (an edit;
/// not a change that isn't one, such as type laid out again, which only shows it again), with the
/// params it was opened with and the same objects without settings. `true` when what it shows
/// changed (started again, shown again, or closed).
fn follow_document(s: &mut Session) -> Result<bool> {
    let Some(st) = s.active() else { return Ok(false) };
    let (uid, doc) = (st.uid, st.doc.clone());
    let Some(l) = s.picabia.physics.as_deref() else { return Ok(false) };
    if l.doc_uid != uid || Arc::ptr_eq(&l.base, &doc) {
        return Ok(false);
    }
    if st.same_content(&l.base) {
        if let Some(l) = s.picabia.physics.as_deref_mut() {
            l.base = doc;
        }
        // Shown again from the new document.
        return Ok(true);
    }
    // The artboard is the one it stood on, if the document still has it.
    let board = l.artboard.filter(|i| *i < doc.artboards.len());
    let p = json!({"gravityX": l.params.gravity.x, "gravityY": l.params.gravity.y, "walls": l.params.walls.is_some(), "artboard": board});
    let playing = l.playing;
    let extra = l.defaulted.clone();
    match build(s, &p, "physics.session.open", Some(&extra)) {
        Ok(mut fresh) => {
            fresh.playing = playing;
            fresh.notes.insert(0, "the document changed: the simulation started again from the new rest pose".into());
            fresh.refresh();
            s.picabia.physics = Some(Box::new(fresh));
        }
        // Nothing moves any more (the bodies were deleted, their settings cleared): it closes.
        Err(_) => s.picabia.physics = None,
    }
    Ok(true)
}

/// The most steps one command may take: phase 1's body-steps budget over what a step costs.
pub(crate) fn step_budget(world: &World) -> u32 {
    let n = sim::MAX_BODY_STEPS / u64::try_from(world.cost()).unwrap_or(u64::MAX).max(1);
    u32::try_from(n).unwrap_or(MAX_SESSION_STEPS).clamp(1, MAX_SESSION_STEPS)
}

fn summary(l: &Live) -> Value {
    let soft = l.bodies.iter().filter(|b| b.mesh.is_some()).count();
    json!({
        "bodies": l.ids.len(),
        "soft": soft,
        "steps": l.sim.step(),
        "skipped": l.skipped,
        "warnings": l.warnings,
        "notes": l.notes,
        "artboard": l.artboard,
    })
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let l = build(s, p, "physics.session.open", None)?;
    let out = summary(&l);
    s.picabia.physics = Some(Box::new(l));
    bump(s);
    Ok(out)
}

fn close(s: &mut Session, _: &Value) -> Result<Value> {
    if live(s).is_some() {
        s.picabia.physics = None;
        bump(s);
    }
    Ok(Value::Null)
}

fn info(s: &mut Session, _: &Value) -> Result<Value> {
    if follow_document(s)? {
        bump(s);
    }
    let Some(l) = live(s) else { return Ok(json!({"open": false})) };
    let kinds: Vec<Value> = l
        .ids
        .iter()
        .zip(&l.bodies)
        .map(|(id, b)| json!({"id": id.0, "kind": if b.mesh.is_some() { "soft" } else { b.settings.body_type.as_str() }}))
        .collect();
    Ok(json!({
        "open": true,
        "step": l.sim.step(),
        "seconds": f64::from(l.sim.step()) / sim::HZ,
        "playing": l.playing,
        "settled": l.sim.world().settled(),
        "bodies": kinds,
        "events": l.sim.events().len(),
        "notes": l.notes,
        "skipped": l.skipped,
        "warnings": l.warnings,
        "artboard": l.artboard,
    }))
}

/// Open a session when none is open in the active document.
fn ensure(s: &mut Session, p: &Value, cmd: &str) -> Result<()> {
    follow_document(s)?;
    if live(s).is_none() {
        let l = build(s, p, cmd, None)?;
        s.picabia.physics = Some(Box::new(l));
    }
    Ok(())
}

fn play(s: &mut Session, p: &Value) -> Result<Value> {
    ensure(s, p, "physics.play")?;
    if let Some(l) = live_mut(s) {
        l.playing = true;
        l.owed = 0.0;
    }
    bump(s);
    Ok(Value::Null)
}

fn pause(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(l) = live_mut(s) {
        l.playing = false;
    }
    bump(s);
    Ok(Value::Null)
}

fn reset(s: &mut Session, _: &Value) -> Result<Value> {
    follow_document(s)?;
    if let Some(l) = live_mut(s) {
        l.sim.reset();
        l.playing = false;
    }
    bump(s);
    Ok(Value::Null)
}

fn step_param(p: &Value, key: &str, default: f64, cmd: &str) -> Result<u32> {
    check_numbers(p, &[key], cmd)?;
    let v = number_or(p, key, default, cmd)?;
    if !v.is_finite() || v < 0.0 || v.fract() != 0.0 || v > f64::from(MAX_SESSION_STEPS) {
        return Err(bad(cmd, format!("`{key}` must be a whole number of steps, 0-{MAX_SESSION_STEPS}")));
    }
    Ok(v as u32)
}

fn seek(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.seek";
    if p.get("step").is_none_or(Value::is_null) {
        return Err(bad(CMD, "`step` is required"));
    }
    let step = step_param(p, "step", 0.0, CMD)?;
    follow_document(s)?;
    let Some(l) = live_mut(s) else { return Err(bad(CMD, "no simulation is open")) };
    // Forward, at most the body-steps budget in one go.
    let most = l.sim.step().saturating_add(step_budget(l.sim.world()));
    let capped = step > most;
    l.sim.seek(step.min(most));
    let out = json!({"step": l.sim.step(), "seconds": f64::from(l.sim.step()) / sim::HZ, "settled": l.sim.world().settled(), "capped": capped});
    bump(s);
    Ok(out)
}

fn to_rest(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.toRest";
    let max = step_param(p, "steps", f64::from(DEFAULT_TO_REST_STEPS), CMD)?.max(1);
    ensure(s, p, CMD)?;
    let Some(l) = live_mut(s) else { return Err(bad(CMD, "no simulation is open")) };
    l.playing = false;
    let budget = step_budget(l.sim.world());
    l.sim.to_rest(max.min(budget));
    let out = json!({"step": l.sim.step(), "seconds": f64::from(l.sim.step()) / sim::HZ, "settled": l.sim.world().settled(), "capped": max > budget});
    bump(s);
    Ok(out)
}

/// A point param: `{x, y}` or `[x, y]`, finite.
fn point(p: &Value, key: &str, cmd: &str) -> Result<Option<vectorcraft_geom::Point>> {
    let v = match p.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let (x, y) = match v {
        Value::Array(a) if a.len() == 2 => (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)),
        Value::Object(o) => (o.get("x").and_then(Value::as_f64), o.get("y").and_then(Value::as_f64)),
        _ => (None, None),
    };
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Ok(Some(vectorcraft_geom::Point::new(x, y))),
        _ => Err(bad(cmd, format!("`{key}` must be {{x, y}} or [x, y], finite numbers within ±10,000,000"))),
    }
}

fn poke(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.poke";
    let at = point(p, "point", CMD)?;
    let to = point(p, "to", CMD)?;
    let release = switch_or(p, "release", false, CMD)?;
    let id = p.get("id").filter(|v| !v.is_null()).map(|v| v.as_u64().map(NodeId).ok_or_else(|| bad(CMD, "`id` must be an object id"))).transpose()?;
    follow_document(s)?;
    let Some(l) = live_mut(s) else { return Err(bad(CMD, "no simulation is open")) };
    if l.sim.events().len() >= MAX_EVENTS {
        return Err(bad(CMD, format!("at most {MAX_EVENTS} pokes and changes are recorded in one simulation")));
    }
    let full = || bad(CMD, format!("at most {MAX_EVENTS} pokes and changes are recorded in one simulation"));
    if let (Some(id), Some(at)) = (id, at) {
        let body = l.body_of(id).ok_or_else(|| bad(CMD, format!("object {} isn't in the simulation", id.0)))?;
        if !l.sim.world().movable(body) {
            return Err(bad(CMD, format!("object {} doesn't move (static, kinematic or out of the world)", id.0)));
        }
        if !l.sim.record(Event::Grab { body, at }) {
            return Err(full());
        }
    } else if id.is_some() || at.is_some() {
        return Err(bad(CMD, "grabbing takes both `id` and `point`"));
    }
    if let Some(to) = to {
        if l.sim.held().is_none() {
            return Err(bad(CMD, "nothing is grabbed: pass `id` and `point` first"));
        }
        if !l.sim.record(Event::DragTo { to }) {
            return Err(full());
        }
    }
    if release && !l.sim.record(Event::Release) {
        return Err(full());
    }
    let step = l.sim.step();
    bump(s);
    Ok(json!({"step": step}))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.apply";
    follow_document(s)?;
    let Some(l) = s.picabia.physics.take() else { return Err(bad(CMD, "no simulation is open")) };
    if s.active().is_none_or(|st| st.uid != l.doc_uid) {
        s.picabia.physics = Some(l);
        return Err(bad(CMD, "the simulation is open in another document"));
    }
    let poses = l.poses();
    let items: Vec<Baking> = poses.iter().map(|(id, pose, mesh)| Baking { id: *id, pose, mesh: *mesh }).collect();
    let changed = match bake(s, "Apply Physics", p, &items) {
        Ok(c) => c,
        Err(e) => {
            // The session stays: Apply can be tried again.
            drop(items);
            drop(poses);
            s.picabia.physics = Some(l);
            return Err(e);
        }
    };
    bump(s);
    Ok(json!({"changed": changed.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// Advance the playing session by `dt` seconds of wall time (at most [`MAX_STEPS_PER_TICK`]
/// steps). The app calls it every frame; `true` when the session moved (redraw).
pub fn tick(s: &mut Session, dt: f64) -> bool {
    match follow_document(s) {
        Err(_) => return false,
        // Started again after an edit: redraw.
        Ok(true) => {
            bump(s);
            return true;
        }
        Ok(false) => {}
    }
    let Some(l) = live_mut(s) else { return false };
    if !l.playing {
        return false;
    }
    let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
    l.owed += dt;
    let due = (l.owed * sim::HZ).floor();
    let n = (due.max(0.0) as u32).min(MAX_STEPS_PER_TICK);
    if n == 0 {
        return false;
    }
    l.owed = (l.owed - f64::from(n) / sim::HZ).min(1.0 / sim::HZ);
    let taken = l.sim.advance(n);
    if taken == 0 {
        l.playing = false;
    }
    bump(s);
    taken > 0
}

/// Whether a session is open and playing in the active document (the app keeps repainting).
pub fn playing(s: &Session) -> bool {
    live(s).is_some_and(|l| l.playing)
}

/// The shown document follows the session, and views redraw: the document's revision moves (the
/// session isn't part of the document).
fn bump(s: &mut Session) {
    if let Some(l) = live_mut(s) {
        l.refresh();
    }
    if let Ok(st) = s.doc_mut() {
        st.revision += 1;
    }
}

/// What the transport bar shows of the active document's session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarState {
    /// The current step (60 a second).
    pub step: u32,
    /// The furthest step reached (the scrubber's end).
    pub furthest: u32,
    pub playing: bool,
    /// Every body rests.
    pub settled: bool,
    /// Bodies in the session.
    pub bodies: usize,
}

/// The transport bar's state, when a session is open in the active document.
pub fn bar_state(s: &Session) -> Option<BarState> {
    let l = live(s)?;
    Some(BarState {
        step: l.sim.step(),
        furthest: l.furthest.max(l.sim.step()),
        playing: l.playing,
        settled: l.sim.world().settled(),
        bodies: l.ids.len(),
    })
}

/// The document the canvas draws while a session is open in the active document: the document at
/// rest with each moved body's clone (its pose) above its object, which is dimmed. Never saved,
/// exported or hit-tested: the real document is unchanged until Apply.
pub fn display_doc(s: &Session) -> Option<Arc<Document>> {
    // Built from the document it shows: after an edit, until the session follows it (the next
    // tick), the document itself is drawn, never a stale copy.
    let st = s.active()?;
    live(s).filter(|l| Arc::ptr_eq(&l.base, &st.doc)).and_then(|l| l.shown.clone())
}

/// A body closer than this to its rest pose (points) hasn't moved.
pub const STILL: f64 = 1e-3;

/// How much of an original shows under its clone.
pub const DIMMED: f32 = 0.25;

impl Live {
    /// Recompute the shown document for the current step.
    pub(crate) fn refresh(&mut self) {
        self.furthest = self.furthest.max(self.sim.step());
        let mut doc = (*self.base).clone();
        let poses = self.poses();
        let paths = doc.index_paths(self.ids.iter().copied());
        // Deepest and last first, so inserting a clone keeps the other paths valid.
        let mut work: Vec<(Vec<usize>, vectorcraft_doc::Node)> = Vec::new();
        for (id, pose, mesh) in &poses {
            let (Some(path), Some(n)) = (paths.get(id), self.base.node(*id)) else { continue };
            let posed = match (pose, mesh) {
                (Pose::Rigid(Some(xf)), _) => {
                    let mut c = n.clone();
                    // As Apply moves it (Transform Pattern Tiles).
                    let sc = vectorcraft_doc::Scaling { patterns: self.patterns, ..vectorcraft_doc::Scaling::default() };
                    crate::cmd::object::transform_node(&mut c, *xf, sc, true);
                    Some(c)
                }
                (Pose::Soft(p), Some(mesh)) => vectorcraft_physics::pose::pose_node(n, mesh, p),
                _ => None,
            };
            if let Some(c) = posed.filter(|c| c != n) {
                work.push((path.clone(), c));
            }
        }
        work.sort_by(|a, b| b.0.cmp(&a.0));
        for (path, clone) in work {
            let Some((&at, parent)) = path.split_last() else { continue };
            if parent.is_empty() {
                continue;
            }
            let clone = doc.reid(&clone);
            if let Some(n) = doc.node_mut_at(&path) {
                n.opacity *= DIMMED;
            }
            if let Some(children) = doc.node_mut_at(parent).and_then(|p| p.children_mut()) {
                let at = (at + 1).min(children.len());
                children.insert(at, Arc::new(clone));
            }
        }
        self.shown = Some(Arc::new(doc));
    }
}

/// The moving body whose shown pose is under `at` (document space), topmost first: a soft body by
/// its particles' bounds grown by their radius, a rigid one by its object's bounds where it is now.
pub fn body_at(s: &Session, at: vectorcraft_geom::Point) -> Option<NodeId> {
    let l = live(s)?;
    let world = l.sim.world();
    let poses = l.poses();
    for (i, (id, pose, mesh)) in poses.iter().enumerate().rev() {
        if !world.movable(i) {
            continue;
        }
        let r = match (pose, mesh) {
            (Pose::Soft(p), Some(m)) => {
                let Some(first) = p.first() else { continue };
                let r = p.iter().fold(vectorcraft_geom::Rect::from_points(*first, *first), |r, q| r.union_pt(*q));
                r.inflate(m.radius + m.spacing / 2.0, m.radius + m.spacing / 2.0)
            }
            (Pose::Rigid(xf), _) => {
                let Some(b) = l.base.node(*id).and_then(|n| n.geometric_bounds()) else { continue };
                xf.map_or(b, |xf| xf.transform_rect_bbox(b))
            }
            _ => continue,
        };
        if r.contains(at) {
            return Some(*id);
        }
    }
    None
}

/// Whether a body built as `new` needs to start again from its rest pose rather than take `new`'s
/// settings in the running world: another kind of body, collider or mass, a new mesh (spacing,
/// cloth to cells, self-contact, pins), or a soft body's gravity or drag (rapier keeps those per
/// particle from when the body was made).
fn needs_restart(old: &WorldBody, new: &WorldBody) -> bool {
    let (a, b) = (&old.settings, &new.settings);
    if a.body_type != b.body_type || a.collider != b.collider || a.mass != b.mass || old.mesh.is_some() != new.mesh.is_some() {
        return true;
    }
    match (a.soft_body(), b.soft_body()) {
        (Some(x), Some(y)) => {
            x.needs_new_mesh(&y) || a.gravity_scale != b.gravity_scale || a.linear_damping != b.linear_damping || old.pins != new.pins
        }
        _ => false,
    }
}

/// `physics.set` changed the settings of `ids` in the active document (which was `before`): a
/// session open on `before` takes them at its next step, recorded as events (so scrubbing back
/// before the change replays the old settings): material, bounce, friction and damping retune the
/// running body; a change that needs a new mesh or another kind of body restarts that body from its
/// rest pose, with a note. When the bodies taking part change (settings turned on or off), the
/// session starts again as after any edit.
pub(crate) fn settings_changed(s: &mut Session, before: &Arc<Document>, ids: &[NodeId]) {
    let Some(l) = live(s) else { return };
    if !Arc::ptr_eq(&l.base, before) {
        return;
    }
    let board = l.artboard;
    let p = json!({"gravityX": l.params.gravity.x, "gravityY": l.params.gravity.y, "walls": l.params.walls.is_some(), "artboard": board});
    let extra = l.defaulted.clone();
    let Ok(g) = gather(s, &p, "physics.set", Some(&extra)) else { return };
    let Ok(doc) = s.doc().map(|st| st.doc.clone()) else { return };
    let Some(l) = live_mut(s) else { return };
    if g.ids != l.ids {
        return;
    }
    for id in ids {
        let Some(i) = l.body_of(*id) else { continue };
        let (Some(old), Some(new)) = (l.bodies.get(i), g.bodies.get(i)) else { continue };
        let event = if needs_restart(old, new) {
            l.notes.push(format!("object {}: its new settings need a new body: it started again from its rest pose", id.0));
            Event::Restart { body: i, input: Box::new(new.clone()) }
        } else {
            Event::Retune { body: i, settings: Box::new(new.settings.clone()) }
        };
        if !l.sim.record(event) {
            // The log is full: the session starts again with the new settings, as after an edit.
            l.notes.push(format!("at most {MAX_EVENTS} changes are recorded: the simulation starts again with the new settings"));
            return;
        }
        if let Some(b) = l.bodies.get_mut(i) {
            *b = new.clone();
        }
    }
    l.base = doc;
    bump(s);
}
