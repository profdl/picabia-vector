//! PICABIA: physics commands (`physics.*`), phase 1: drop-and-settle.
//!
//! An object's physics settings live in its `picabia.physics` foreign attribute (see
//! `vectorcraft_physics`), so save and reopen, copy and paste, duplicate and undo carry them.
//! `physics.simulate` runs the world synchronously to rest (or a step cap) and moves each dynamic
//! body's object by its rigid motion as the transform commands do, as one undo step. There is no
//! clock: nothing moves until Simulate runs (`docs/picabia/physics.md`).

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, Vec2};
use vectorcraft_physics::mesh::{MAX_PARTICLES_PER_RUN, SoftMesh};
use vectorcraft_physics::pose::Pose;
use vectorcraft_physics::world::{World, WorldBody, WorldParams, moves};
use vectorcraft_physics::{BodyInput, BodyType, Settings, SimParams, sim};

use super::{CommandSpec, bad, check_numbers, cmd, has_doc, has_selection, number_or, switch_or, targets};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    let mut v = base_specs();
    v.extend(session::specs());
    v.extend(pins::specs());
    v
}

fn base_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "physics.set",
            "Physics Settings",
            [],
            None,
            "{ids?|id?: the objects (default: the selection), props?: {enabled?, bodyType? (dynamic|static|kinematic|soft), colliderType? (auto|precise|fast|box), gravityScale? (0-2), restitution? (0-1), friction? (0-1), mass? (kg added to the body's own, 0-10000), angularDamping? (0-20), linearDamping? (0-20), sensor?, soft?: {preset? (cloth|jelly|rubber), stiffness? (0-1), bend? (0-1), shapeKeep? (0-1), damping? (0-1), resolution? (particle spacing, 2-100 pt), selfCollide?, pins?: [{u, v: where on the object, fractions of its bounds, attachTo?: id}]; a field set to null follows the preset again}}} give the objects physics settings (missing ones: the defaults, dynamic) or change them, as one undo step; out-of-range values are clamped and wrong ones ignored, with warnings → {ids, settings: the first object's, warnings}",
            has_selection,
            set
        ),
        cmd!("physics.clear", "Clear Physics", [], None, "{ids?|id?} remove the objects' physics settings, as one undo step", has_selection, clear),
        cmd!(
            query "physics.get",
            "Physics Settings Info",
            [],
            None,
            "{id?: the object (default: the first selected)} → {id, settings: {…} | null (none), warnings, error?: why its settings can't be read}",
            has_doc,
            get
        ),
        cmd!(
            "physics.simulate",
            "Simulate Physics",
            [],
            None,
            "{steps? (the step cap at 60 per second, 1-20000; default 3000), stopWhenSettled? (default true: stop once every dynamic body sleeps), gravityX?, gravityY? (pt/s², ±100000; default 0, 981: 9.81 m/s² at 100 pt per metre, down the page), walls? (default true: the artboard's edges hold the bodies), artboard? (its index; default: the one under the dynamic bodies, else the first)} drop and settle: every visible object with physics settings turned on takes part (an object inside one with settings moves with it), and selected objects without settings as dynamic bodies with the defaults; locked objects stay put as obstacles; each dynamic body's object is moved and turned as the transform commands do, as one undo step → {bodies, moved: [ids], steps, seconds, settled, lost: [ids that fell out of the world, left where they were], skipped: [{id, reason}], warnings: [{id, message}], notes, artboard}",
            has_doc,
            simulate
        ),
    ]
}

/// The objects a command edits: `ids`/`id` or the selection; layers are refused.
fn objects(cmd: &str, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = targets(s, p).map_err(|e| bad(cmd, e.to_string()))?;
    if ids.is_empty() {
        return Err(bad(cmd, "select an object or pass `ids`"));
    }
    let doc = &s.doc()?.doc;
    for id in &ids {
        let n = doc.node(*id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
        if n.is_layer() {
            return Err(bad(cmd, format!("{} is a layer; physics goes on objects", id.0)));
        }
    }
    Ok(ids)
}

fn props_of<'a>(cmd: &str, p: &'a Value) -> Result<Option<&'a Map<String, Value>>> {
    match p.get("props") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(bad(cmd, "`props` must be an object")),
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.set";
    let ids = objects(CMD, s, p)?;
    let empty = Map::new();
    let props = props_of(CMD, p)?.unwrap_or(&empty);
    let doc = &s.doc()?.doc;
    let mut warnings: Vec<Value> = Vec::new();
    let mut changes: Vec<(NodeId, Settings)> = Vec::new();
    for id in &ids {
        let n = doc.node(*id).ok_or_else(|| bad(CMD, format!("no object with id {}", id.0)))?;
        let (mut settings, old) = match vectorcraft_physics::node_settings(n) {
            None => (Settings::default(), None),
            Some(Ok((st, _))) => (st.clone(), Some(st)),
            Some(Err(e)) => {
                warnings.push(json!({"id": id.0, "message": format!("its settings couldn't be read ({e}): replaced")}));
                (Settings::default(), None)
            }
        };
        for w in settings.apply(props, false) {
            warnings.push(json!({"id": id.0, "message": w}));
        }
        if old.as_ref() != Some(&settings) {
            changes.push((*id, settings));
        }
    }
    let first = ids.first().and_then(|id| changes.iter().find(|(c, _)| c == id).map(|(_, st)| st.to_value()));
    let first = match first {
        Some(v) => v,
        None => ids
            .first()
            .and_then(|id| doc.node(*id))
            .and_then(vectorcraft_physics::node_settings)
            .and_then(|r| r.ok())
            .map_or(Value::Null, |(st, _)| st.to_value()),
    };
    // Nothing changes: no undo step.
    if !changes.is_empty() {
        let before = s.doc()?.doc.clone();
        s.edit("Physics Settings", |d, _| {
            for (id, st) in &changes {
                if let Some(n) = d.node_mut(*id) {
                    vectorcraft_physics::set_node_settings(n, Some(st));
                }
            }
            Ok(())
        })?;
        // A simulation playing in this document takes the change at its next step (PIC31.7).
        session::settings_changed(s, &before, &changes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    }
    Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "settings": first, "warnings": warnings}))
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.clear";
    let ids = objects(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let with: Vec<NodeId> = ids.into_iter().filter(|id| doc.node(*id).is_some_and(vectorcraft_physics::has_physics)).collect();
    if with.is_empty() {
        return Ok(Value::Null);
    }
    s.edit("Clear Physics", |d, _| {
        for id in with {
            if let Some(n) = d.node_mut(id) {
                vectorcraft_physics::set_node_settings(n, None);
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.get";
    let id = targets(s, p).map_err(|e| bad(CMD, e.to_string()))?.first().copied().ok_or_else(|| bad(CMD, "select an object or pass `id`"))?;
    let n = s.doc()?.doc.node(id).ok_or_else(|| bad(CMD, format!("no object with id {}", id.0)))?;
    Ok(match vectorcraft_physics::node_settings(n) {
        None => json!({"id": id.0, "settings": null, "warnings": []}),
        Some(Ok((st, w))) => json!({"id": id.0, "settings": st.to_value(), "warnings": w}),
        Some(Err(e)) => json!({"id": id.0, "settings": null, "warnings": [], "error": e}),
    })
}

/// An object taking part in a simulation.
struct Found {
    id: NodeId,
    settings: Settings,
    warnings: Vec<String>,
}

/// Collect the bodies under `n`: an object with settings turned on is one body (what it holds
/// moves with it); a selected object without settings is a dynamic body with the defaults.
/// Hidden objects are left out; locked ones (or on a locked layer) stay put.
fn collect(n: &Node, locked: bool, selected: &HashSet<NodeId>, depth: usize, out: &mut Vec<Found>, skipped: &mut Vec<Value>) {
    if !n.visible || depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    let locked = locked || n.locked;
    if !n.is_layer() {
        let lock = |mut st: Settings, mut w: Vec<String>| {
            if locked && moves(&st.body_type) {
                st.body_type = BodyType::Static;
                w.push("locked: stays put".into());
            }
            (st, w)
        };
        match vectorcraft_physics::node_settings(n) {
            Some(Ok((st, w))) if st.enabled => {
                let (settings, warnings) = lock(st, w);
                out.push(Found { id: n.id, settings, warnings });
                return;
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                skipped.push(json!({"id": n.id.0, "reason": e}));
                return;
            }
            None if selected.contains(&n.id) => {
                let (settings, warnings) = lock(Settings::default(), vec![]);
                out.push(Found { id: n.id, settings, warnings });
                return;
            }
            None => {}
        }
    }
    if let NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } = &n.kind {
        for c in children {
            collect(c, locked, selected, depth + 1, out, skipped);
        }
    }
}

/// The artboard the walls stand on: `artboard` when given, else the one holding the centre of the
/// dynamic bodies' bounds, else the first.
fn pick_artboard(cmd: &str, doc: &Document, p: &Value, dynamic: Option<Rect>) -> Result<Option<usize>> {
    match p.get("artboard").filter(|v| !v.is_null()) {
        Some(v) => {
            let i = v.as_u64().ok_or_else(|| bad(cmd, format!("`artboard` must be an artboard index, not {v}")))?;
            let i = usize::try_from(i).map_err(|_| bad(cmd, "no such artboard"))?;
            if i >= doc.artboards.len() {
                return Err(bad(cmd, format!("no artboard {i} (the document has {})", doc.artboards.len())));
            }
            Ok(Some(i))
        }
        None => {
            if doc.artboards.is_empty() {
                return Ok(None);
            }
            let at = dynamic.map(|r| r.center()).and_then(|c| doc.artboards.iter().position(|a| a.rect.contains(c)));
            Ok(Some(at.unwrap_or(0)))
        }
    }
}

/// The bodies a simulation takes, shared by Simulate and the live session.
pub(crate) struct Gathered {
    /// The bodies' objects, in the world's order.
    pub(crate) ids: Vec<NodeId>,
    pub(crate) bodies: Vec<WorldBody>,
    pub(crate) skipped: Vec<Value>,
    pub(crate) warnings: Vec<Value>,
    /// Objects without settings taking part as dynamic bodies with the defaults (they were
    /// selected).
    pub(crate) defaulted: Vec<NodeId>,
    pub(crate) params: WorldParams,
    /// The artboard the walls stand on.
    pub(crate) artboard: Option<usize>,
}

/// The pin `pin` of a soft body whose rest mesh is `mesh` and whose object's bounds are `b`: the
/// nearest particle to its place.
fn pin_particle(mesh: &SoftMesh, b: Rect, pin: &vectorcraft_physics::Pin) -> Option<u32> {
    let at = vectorcraft_geom::Point::new(b.x0 + pin.u * b.width(), b.y0 + pin.v * b.height());
    mesh.nearest(at).and_then(|i| u32::try_from(i).ok())
}

/// Read the params (`gravityX`, `gravityY`, `walls`, `artboard`) and the bodies of the active
/// document (see `physics.simulate`'s params doc): rigid bodies' outlines, soft bodies' rest meshes
/// (within the run's particle budget) and pins. Objects without settings in `extra` (the
/// selection, or a session's own when it starts again) take part as dynamic bodies.
pub(crate) fn gather(s: &Session, p: &Value, cmd: &str, extra: Option<&HashSet<NodeId>>) -> Result<Gathered> {
    check_numbers(p, &["gravityX", "gravityY"], cmd)?;
    let gx = number_or(p, "gravityX", 0.0, cmd)?;
    let gy = number_or(p, "gravityY", sim::DEFAULT_GRAVITY, cmd)?;
    if !gx.is_finite() || !gy.is_finite() || gx.abs() > sim::MAX_GRAVITY || gy.abs() > sim::MAX_GRAVITY {
        return Err(bad(cmd, format!("gravity must be within ±{} pt/s²", sim::MAX_GRAVITY)));
    }
    let walls = switch_or(p, "walls", true, cmd)?;
    let st = s.doc()?;
    let doc = &st.doc;
    let selected: HashSet<NodeId> = extra.cloned().unwrap_or_else(|| st.selection.objects.iter().copied().collect());
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for layer in &doc.layers {
        collect(layer, false, &selected, 0, &mut found, &mut skipped);
    }
    // At most `MAX_BODIES` take part, moving ones first (in layer order), before any outline is
    // read: the rest are reported.
    if found.len() > sim::MAX_BODIES {
        let (mut keep, rest): (Vec<Found>, Vec<Found>) = found.into_iter().partition(|f| moves(&f.settings.body_type));
        let mut over: Vec<Found> = keep.split_off(keep.len().min(sim::MAX_BODIES));
        let room = sim::MAX_BODIES - keep.len();
        let mut rest = rest;
        over.extend(rest.split_off(rest.len().min(room)));
        keep.extend(rest);
        for f in over {
            skipped.push(json!({"id": f.id.0, "reason": format!("over the {}-body limit", sim::MAX_BODIES)}));
        }
        found = keep;
    }
    let symbols = doc.symbol_boxes();
    let mut ids = Vec::new();
    let mut bodies: Vec<WorldBody> = Vec::new();
    let mut pins: Vec<Vec<vectorcraft_physics::Pin>> = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut budget = MAX_PARTICLES_PER_RUN;
    let mut defaulted = Vec::new();
    for f in found {
        let Some(n) = doc.node(f.id) else { continue };
        let shape = match vectorcraft_physics::body_shape(n, Some(&symbols)) {
            Ok(shape) => shape,
            Err(reason) => {
                skipped.push(json!({"id": f.id.0, "reason": reason}));
                continue;
            }
        };
        let mut notes: Vec<String> = f.warnings.clone();
        notes.extend(shape.notes.iter().cloned());
        let (mesh, body_pins) = match f.settings.soft_body() {
            None => (None, Vec::new()),
            Some(soft) => match vectorcraft_physics::mesh::soft_mesh(&shape, &soft, budget) {
                Ok(mesh) => {
                    budget = budget.saturating_sub(mesh.rest.len());
                    notes.extend(mesh.notes.iter().cloned());
                    (Some(mesh), soft.pins.clone())
                }
                Err(reason) => {
                    skipped.push(json!({"id": f.id.0, "reason": reason}));
                    continue;
                }
            },
        };
        warnings.extend(notes.iter().map(|m| json!({"id": f.id.0, "message": m})));
        if doc.node(f.id).is_some_and(|n| !vectorcraft_physics::has_physics(n)) {
            defaulted.push(f.id);
        }
        ids.push(f.id);
        pins.push(body_pins);
        bodies.push(WorldBody { settings: f.settings, shape, mesh, pins: Vec::new() });
    }
    // Pins: each takes its nearest particle; attached to a body in this simulation by its index.
    for (i, list) in pins.iter().enumerate() {
        let Some(b) = bodies.get(i) else { continue };
        let Some(mesh) = &b.mesh else { continue };
        let bounds = b.shape.bounds;
        let mut resolved = Vec::new();
        for pin in list {
            let Some(particle) = pin_particle(mesh, bounds, pin) else { continue };
            let to = match pin.attach_to {
                None => None,
                Some(target) => match ids.iter().position(|id| id.0 == target) {
                    Some(k) if k != i && bodies.get(k).is_some_and(|t| t.mesh.is_none()) => Some(k),
                    _ => {
                        warnings.push(json!({"id": ids.get(i).map_or(0, |id| id.0), "message": format!("a pin is attached to object {target}, which isn't a rigid body in this simulation: pinned in place")}));
                        None
                    }
                },
            };
            resolved.push((particle, to));
        }
        if let Some(b) = bodies.get_mut(i) {
            b.pins = resolved;
        }
    }
    let moving = bodies.iter().filter(|b| moves(&b.settings.body_type)).map(|b| b.shape.bounds).reduce(|a, b| a.union(b));
    let artboard = pick_artboard(cmd, doc, p, moving)?;
    let wall_rect = if walls { artboard.and_then(|i| doc.artboards.get(i)).map(|a| a.rect) } else { None };
    Ok(Gathered {
        ids,
        bodies,
        skipped,
        warnings,
        defaulted,
        params: WorldParams { gravity: Vec2::new(gx, gy), walls: wall_rect },
        artboard: if walls { artboard } else { None },
    })
}

fn simulate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.simulate";
    check_numbers(p, &["steps"], CMD)?;
    let steps = number_or(p, "steps", f64::from(sim::DEFAULT_MAX_STEPS), CMD)?;
    if !steps.is_finite() || steps < 1.0 || steps.fract() != 0.0 {
        return Err(bad(CMD, format!("`steps` must be a whole number of steps, 1-{}", sim::MAX_STEPS)));
    }
    let steps = steps.min(f64::from(sim::MAX_STEPS)) as u32;
    let settle = switch_or(p, "stopWhenSettled", true, CMD)?;
    let g = gather(s, p, CMD, None)?;
    if !g.bodies.iter().any(|b| moves(&b.settings.body_type)) {
        return Err(EngineError::Disabled(
            CMD.into(),
            "nothing to drop: select objects, or give objects physics settings (Object ▸ Physics ▸ Add Physics)".into(),
        ));
    }
    let (poses, ran, settled, lost, notes) = if g.bodies.iter().any(|b| b.mesh.is_some()) {
        // Soft bodies: the live world, run to rest (phase 2), within the body-steps budget.
        let (mut world, mut notes) = World::new(&g.bodies, &g.params);
        let budget = session::step_budget(&world);
        if steps > budget {
            notes.push(format!("{} body-steps at most: {budget} steps", sim::MAX_BODY_STEPS));
        }
        let mut ran = 0;
        while ran < steps.min(budget) {
            world.step();
            ran += 1;
            if settle && world.settled() {
                break;
            }
        }
        let lost: Vec<u64> = world.lost().iter().filter_map(|i| g.ids.get(*i)).map(|id| id.0).collect();
        (world.poses(), ran, world.settled(), lost, notes)
    } else {
        // Rigid bodies only: phase 1's drop, unchanged.
        let inputs: Vec<BodyInput> = g.bodies.iter().map(|b| BodyInput { settings: b.settings.clone(), shape: b.shape.clone() }).collect();
        let params = SimParams { gravity: g.params.gravity, walls: g.params.walls, max_steps: steps, stop_when_settled: settle };
        let out = vectorcraft_physics::simulate(&inputs, &params);
        let lost: Vec<u64> = g.ids.iter().zip(&out.outcomes).filter(|(_, o)| o.lost).map(|(id, _)| id.0).collect();
        (out.outcomes.iter().map(|o| Pose::Rigid(o.moved)).collect(), out.steps, out.settled, lost, out.notes)
    };
    let items: Vec<apply::Baking> =
        g.ids.iter().zip(&poses).zip(&g.bodies).map(|((id, pose), b)| apply::Baking { id: *id, pose, mesh: b.mesh.as_ref() }).collect();
    let moved = apply::bake(s, "Simulate Physics", p, &items)?;
    Ok(json!({
        "bodies": g.bodies.len(),
        "moved": moved.iter().map(|id| id.0).collect::<Vec<_>>(),
        "steps": ran,
        "seconds": f64::from(ran) / sim::HZ,
        "settled": settled,
        "lost": lost,
        "skipped": g.skipped,
        "warnings": g.warnings,
        "notes": notes,
        "artboard": g.artboard,
    }))
}

#[path = "picabia_physics_apply.rs"]
pub(crate) mod apply;
#[path = "picabia_physics_pins.rs"]
pub mod pins;
#[path = "picabia_physics_session.rs"]
pub mod session;

#[cfg(test)]
#[path = "picabia_physics_apply_tests.rs"]
mod apply_tests;
#[cfg(test)]
#[path = "picabia_physics_pins_tests.rs"]
mod pins_tests;
#[cfg(test)]
#[path = "picabia_physics_session_tests.rs"]
mod session_tests;
#[cfg(test)]
#[path = "picabia_physics_tests.rs"]
mod tests;
