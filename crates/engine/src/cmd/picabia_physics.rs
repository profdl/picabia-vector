//! PICABIA: physics commands (`physics.*`), phase 1: drop-and-settle.
//!
//! An object's physics settings live in its `picabia.physics` foreign attribute (see
//! `vectorcraft_physics`), so save and reopen, copy and paste, duplicate and undo carry them.
//! `physics.simulate` runs the world synchronously to rest (or a step cap) and moves each dynamic
//! body's object by its rigid motion as the transform commands do, as one undo step. There is no
//! clock: nothing moves until Simulate runs (`docs/picabia/physics.md`).

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Rect, Vec2};
use vectorcraft_physics::{BodyInput, BodyType, Settings, SimParams, sim};

use super::{CommandSpec, bad, check_numbers, cmd, has_doc, has_selection, number_or, switch_or, targets};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "physics.set",
            "Physics Settings",
            [],
            None,
            "{ids?|id?: the objects (default: the selection), props?: {enabled?, bodyType? (dynamic|static|kinematic), colliderType? (auto|precise|fast|box), gravityScale? (0-2), restitution? (0-1), friction? (0-1), mass? (kg added to the body's own, 0-10000), angularDamping? (0-20), linearDamping? (0-20), sensor?}} give the objects physics settings (missing ones: the defaults, dynamic) or change them, as one undo step; out-of-range values are clamped and wrong ones ignored, with warnings → {ids, settings: the first object's, warnings}",
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
        s.edit("Physics Settings", |d, _| {
            for (id, st) in &changes {
                if let Some(n) = d.node_mut(*id) {
                    vectorcraft_physics::set_node_settings(n, Some(st));
                }
            }
            Ok(())
        })?;
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
            if locked && st.body_type == BodyType::Dynamic {
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

fn simulate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.simulate";
    check_numbers(p, &["steps", "gravityX", "gravityY"], CMD)?;
    let steps = number_or(p, "steps", f64::from(sim::DEFAULT_MAX_STEPS), CMD)?;
    if !steps.is_finite() || steps < 1.0 || steps.fract() != 0.0 {
        return Err(bad(CMD, format!("`steps` must be a whole number of steps, 1-{}", sim::MAX_STEPS)));
    }
    let steps = steps.min(f64::from(sim::MAX_STEPS)) as u32;
    let gx = number_or(p, "gravityX", 0.0, CMD)?;
    let gy = number_or(p, "gravityY", sim::DEFAULT_GRAVITY, CMD)?;
    if !gx.is_finite() || !gy.is_finite() || gx.abs() > sim::MAX_GRAVITY || gy.abs() > sim::MAX_GRAVITY {
        return Err(bad(CMD, format!("gravity must be within ±{} pt/s²", sim::MAX_GRAVITY)));
    }
    let walls = switch_or(p, "walls", true, CMD)?;
    let settle = switch_or(p, "stopWhenSettled", true, CMD)?;

    let st = s.doc()?;
    let doc = &st.doc;
    let selected: HashSet<NodeId> = st.selection.objects.iter().copied().collect();
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for layer in &doc.layers {
        collect(layer, false, &selected, 0, &mut found, &mut skipped);
    }
    // At most `MAX_BODIES` take part, dynamic ones first (in layer order), before any outline is
    // read: the rest are reported.
    if found.len() > sim::MAX_BODIES {
        let (mut keep, rest): (Vec<Found>, Vec<Found>) = found.into_iter().partition(|f| f.settings.body_type == BodyType::Dynamic);
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
    let mut bodies = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();
    for f in found {
        let Some(n) = doc.node(f.id) else { continue };
        match vectorcraft_physics::body_shape(n, Some(&symbols)) {
            Ok(shape) => {
                warnings.extend(f.warnings.iter().chain(&shape.notes).map(|m| json!({"id": f.id.0, "message": m})));
                ids.push(f.id);
                bodies.push(BodyInput { settings: f.settings, shape });
            }
            Err(reason) => skipped.push(json!({"id": f.id.0, "reason": reason})),
        }
    }
    if !bodies.iter().any(|b| b.settings.body_type == BodyType::Dynamic) {
        return Err(EngineError::Disabled(
            CMD.into(),
            "nothing to drop: select objects, or give objects physics settings (Object ▸ Physics ▸ Add Physics)".into(),
        ));
    }
    let dynamic = bodies.iter().filter(|b| b.settings.body_type == BodyType::Dynamic).map(|b| b.shape.bounds).reduce(|a, b| a.union(b));
    let board = pick_artboard(CMD, doc, p, dynamic)?;
    let wall_rect = if walls { board.and_then(|i| doc.artboards.get(i)).map(|a| a.rect) } else { None };
    let params = SimParams { gravity: Vec2::new(gx, gy), walls: wall_rect, max_steps: steps, stop_when_settled: settle };
    let out = vectorcraft_physics::simulate(&bodies, &params);

    let moves: Vec<(NodeId, vectorcraft_geom::Affine)> = ids.iter().zip(&out.outcomes).filter_map(|(id, o)| Some((*id, o.moved?))).collect();
    let lost: Vec<u64> = ids.iter().zip(&out.outcomes).filter(|(_, o)| o.lost).map(|(id, _)| id.0).collect();
    if !moves.is_empty() {
        let moved_ids: Vec<NodeId> = moves.iter().map(|(id, _)| *id).collect();
        let sc = Scaling { patterns: super::object::transform_patterns(s, p, &moved_ids)?, ..Scaling::default() };
        s.edit("Simulate Physics", |d, _| {
            let paths = d.index_paths(moved_ids.iter().copied());
            for (id, xf) in &moves {
                if let Some(n) = paths.get(id).and_then(|path| d.node_mut_at(path)) {
                    super::object::transform_node(n, *xf, sc, true);
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({
        "bodies": bodies.len(),
        "moved": moves.iter().map(|(id, _)| id.0).collect::<Vec<_>>(),
        "steps": out.steps,
        "seconds": f64::from(out.steps) / sim::HZ,
        "settled": out.settled,
        "lost": lost,
        "skipped": skipped,
        "warnings": warnings,
        "notes": out.notes,
        "artboard": if walls { board.map_or(Value::Null, |i| json!(i)) } else { Value::Null },
    }))
}

#[cfg(test)]
#[path = "picabia_physics_tests.rs"]
mod tests;
