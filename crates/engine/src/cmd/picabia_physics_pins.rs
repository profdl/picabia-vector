//! PICABIA: pins (PIC31.8): `physics.pin` and `physics.unpin`, and where pins show on the canvas.
//!
//! A soft body's pins live in its settings as places on the object (fractions of its geometric
//! bounds, so they survive moving, scaling and editing the path) with an optional object to attach
//! to (a rigid body in the simulation carries the pinned points with it; without one they stay in
//! place). Pin Points pins the anchors chosen with Direct Selection on a soft object (or a group
//! holding them); Attach Points To pins them to the other selected object, which must have physics
//! settings and not be soft. At build time each pin takes the nearest particle.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_physics::soft::{MAX_PINS, Pin};
use vectorcraft_physics::{BodyType, Settings};

use super::session::live;
use crate::cmd::{CommandSpec, bad, cmd, has_doc, switch_or};
use crate::{Result, Session};

/// Pins closer than this (points) to a point given to Unpin are removed.
pub const UNPIN_REACH: f64 = 4.0;
/// The most points one command takes.
const MAX_POINTS: usize = 1_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "physics.pin",
            "Pin Points",
            [],
            None,
            "{id?: the soft object, points?: [[x, y] | {x, y}] (document space; default: the anchors selected with Direct Selection on soft objects), attachTo?: an object id (it must have physics settings and not be soft: the points move with it; it wins over attachToSelected), attachToSelected?: attach to the other selected object with rigid physics settings} pin points of soft bodies, as one undo step ('Pin Points'; none when nothing changes); a point pinned again keeps its attachment unless one is given; at most 64 pins an object → {pins: [{id, count}], warnings}",
            // Enabled with a document: `id` + `points` need no selection; with none chosen the
            // command says what to select.
            has_doc,
            pin
        ),
        cmd!(
            "physics.unpin",
            "Unpin Points",
            [],
            None,
            "{id?, points?: [[x, y] | {x, y}] (pins within 4 pt of each go; with `id` and no points: all of that object's pins; default: the anchors selected with Direct Selection, or every pin of the selected soft objects when no anchor is selected)} remove pins, as one undo step ('Unpin Points') → {pins: [{id, count}]}",
            // Enabled with a document: `id` + `points` need no selection; with none chosen the
            // command says what to select.
            has_doc,
            unpin
        ),
    ]
}

/// The soft body holding object `id`, as the simulation takes bodies: from the outside in, the first
/// object (not a layer) with physics settings turned on is the body; it holds `id` when it is soft.
fn soft_owner(doc: &Document, id: NodeId) -> Option<NodeId> {
    let paths = doc.index_paths([id]);
    let path = paths.get(&id)?;
    let body = (1..=path.len())
        .filter_map(|k| path.get(..k))
        .filter_map(|p| doc.node_at(p))
        .filter(|n| !n.is_layer())
        .find(|n| vectorcraft_physics::node_settings(n).is_some_and(|r| r.is_ok_and(|(st, _)| st.enabled)))?;
    soft_settings(body).map(|_| body.id)
}

/// An object's bounds as the simulation measures them (symbol instances by their art).
fn bounds(doc: &Document, n: &vectorcraft_doc::Node) -> Option<Rect> {
    n.geometric_bounds_with(&doc.symbol_boxes())
}

fn soft_settings(n: &vectorcraft_doc::Node) -> Option<Settings> {
    let (s, _) = vectorcraft_physics::node_settings(n)?.ok()?;
    (s.body_type == BodyType::Soft).then_some(s)
}

/// A point param list: `[[x, y] | {x, y}]`, finite, at most [`MAX_POINTS`].
fn points(p: &Value, cmd: &str) -> Result<Option<Vec<Point>>> {
    let a = match p.get("points") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad(cmd, "`points` must be a list of [x, y] or {x, y}")),
    };
    if a.len() > MAX_POINTS {
        return Err(bad(cmd, format!("at most {MAX_POINTS} points")));
    }
    let mut out = Vec::with_capacity(a.len());
    for v in a {
        let (x, y) = match v {
            Value::Array(xy) if xy.len() == 2 => (xy.first().and_then(Value::as_f64), xy.get(1).and_then(Value::as_f64)),
            Value::Object(o) => (o.get("x").and_then(Value::as_f64), o.get("y").and_then(Value::as_f64)),
            _ => (None, None),
        };
        match (x, y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => out.push(Point::new(x, y)),
            _ => return Err(bad(cmd, format!("`points` holds {v}: each must be [x, y] or {{x, y}}, finite"))),
        }
    }
    Ok(Some(out))
}

/// The points to work on, by soft object: `id` + `points` when given, else the anchors selected
/// with Direct Selection (each on the soft body holding its path).
fn targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<(NodeId, Vec<Point>)>> {
    let st = s.doc()?;
    let doc = &st.doc;
    let given = points(p, cmd)?;
    if let Some(id) = p.get("id").filter(|v| !v.is_null()) {
        let id = NodeId(id.as_u64().ok_or_else(|| bad(cmd, "`id` must be an object id"))?);
        let owner = soft_owner(doc, id)
            .ok_or_else(|| bad(cmd, format!("object {} isn't a soft body (Object ▸ Physics ▸ Make Cloth / Jelly / Rubber)", id.0)))?;
        return Ok(vec![(owner, given.unwrap_or_default())]);
    }
    if let Some(pts) = given {
        return Err(bad(cmd, format!("pass `id` with the {} points", pts.len())));
    }
    let mut out: Vec<(NodeId, Vec<Point>)> = Vec::new();
    for (path_id, anchors) in &st.selection.anchors {
        let Some(owner) = soft_owner(doc, *path_id) else { continue };
        let Some(NodeKind::Path { path, .. }) = doc.node(*path_id).map(|n| &n.kind) else { continue };
        let pts: Vec<Point> = anchors.iter().filter_map(|(sp, a)| path.subpaths.get(*sp)?.anchors.get(*a).map(|a| a.p)).collect();
        match out.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, v)) => v.extend(pts),
            None => out.push((owner, pts)),
        }
    }
    Ok(out)
}

/// `p` as a place on an object with bounds `b`.
fn place(b: Rect, p: Point) -> (f64, f64) {
    let u = if b.width() > 0.0 { (p.x - b.x0) / b.width() } else { 0.5 };
    let v = if b.height() > 0.0 { (p.y - b.y0) / b.height() } else { 0.5 };
    let c = vectorcraft_physics::soft::MAX_PIN_UV;
    (u.clamp(-c, c), v.clamp(-c, c))
}

/// Where `pin` is on an object with bounds `b`.
pub(crate) fn at(b: Rect, pin: &Pin) -> Point {
    Point::new(b.x0 + pin.u * b.width(), b.y0 + pin.v * b.height())
}

/// Store `edits` (object, new soft settings) as one undo step.
fn store(s: &mut Session, label: &str, edits: Vec<(NodeId, Settings)>, warnings: Vec<String>) -> Result<Value> {
    let summary: Vec<Value> = edits.iter().map(|(id, st)| json!({"id": id.0, "count": st.soft.as_ref().map_or(0, |x| x.pins.len())})).collect();
    // Only what changed is stored: no empty undo step, no restart.
    let doc = &s.doc()?.doc;
    let edits: Vec<(NodeId, Settings)> = edits
        .into_iter()
        .filter(|(id, st)| doc.node(*id).and_then(vectorcraft_physics::node_settings).and_then(|r| r.ok()).is_none_or(|(old, _)| old != *st))
        .collect();
    if edits.is_empty() {
        return Ok(json!({"pins": summary, "warnings": warnings}));
    }
    let before = s.doc()?.doc.clone();
    let ids: Vec<NodeId> = edits.iter().map(|(id, _)| *id).collect();
    s.edit(label, |d, _| {
        for (id, st) in &edits {
            if let Some(n) = d.node_mut(*id) {
                vectorcraft_physics::set_node_settings(n, Some(st));
            }
        }
        Ok(())
    })?;
    // A simulation open on the document restarts those bodies with their pins (PIC31.7's path).
    super::session::settings_changed(s, &before, &ids);
    Ok(json!({"pins": summary, "warnings": warnings}))
}

fn pin(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.pin";
    let list = targets(s, p, CMD)?;
    if list.iter().all(|(_, pts)| pts.is_empty()) {
        return Err(bad(CMD, "choose points to pin: anchors of a soft object with Direct Selection, or pass `id` and `points`"));
    }
    let to_selected = switch_or(p, "attachToSelected", false, CMD)?;
    let doc = &s.doc()?.doc;
    let attach = match p.get("attachTo").filter(|v| !v.is_null()) {
        Some(v) => Some(NodeId(v.as_u64().ok_or_else(|| bad(CMD, "`attachTo` must be an object id"))?)),
        None if to_selected => {
            // The first other selected object with rigid physics settings turned on.
            let owners: Vec<NodeId> = list.iter().map(|(o, _)| *o).collect();
            let st = s.doc()?;
            let rigid = |id: &NodeId| {
                doc.node(*id)
                    .and_then(vectorcraft_physics::node_settings)
                    .and_then(|r| r.ok())
                    .is_some_and(|(set, _)| set.enabled && set.body_type != BodyType::Soft)
            };
            let other = st.selection.objects.iter().copied().find(|id| !owners.contains(id) && rigid(id));
            Some(other.ok_or_else(|| bad(CMD, "select the object to attach to as well (it needs physics settings and must not be soft)"))?)
        }
        None => None,
    };
    if let Some(a) = attach {
        let n = doc.node(a).ok_or_else(|| bad(CMD, format!("no object with id {}", a.0)))?;
        match vectorcraft_physics::node_settings(n) {
            Some(Ok((st, _))) if st.body_type != BodyType::Soft => {}
            Some(Ok(_)) => return Err(bad(CMD, "points can't be attached to a soft body: attach them to a rigid one")),
            _ => {
                return Err(bad(
                    CMD,
                    format!("object {} has no physics settings: give it some (Object ▸ Physics ▸ Add Physics) to attach to it", a.0),
                ));
            }
        }
    }
    let mut edits = Vec::new();
    let mut warnings = Vec::new();
    for (owner, pts) in list.into_iter().filter(|(_, pts)| !pts.is_empty()) {
        let Some(n) = doc.node(owner) else { continue };
        let (Some(mut st), Some(b)) = (soft_settings(n), bounds(doc, n)) else { continue };
        let soft = st.soft.get_or_insert_with(Default::default);
        let mut over = 0;
        for q in pts {
            let (u, v) = place(b, q);
            // The same place (within a thousandth of a point) is the same pin.
            let here = at(b, &Pin::new(u, v, None));
            if let Some(same) = soft.pins.iter_mut().find(|x| (at(b, x) - here).hypot() < 1e-3) {
                if let Some(a) = attach {
                    same.attach_to = Some(a.0);
                }
            } else if soft.pins.len() < MAX_PINS {
                soft.pins.push(Pin::new(u, v, attach.map(|a| a.0)));
            } else {
                over += 1;
            }
        }
        if over > 0 {
            warnings.push(format!("object {}: at most {MAX_PINS} pins: {over} points left out", owner.0));
        }
        edits.push((owner, st));
    }
    store(s, "Pin Points", edits, warnings)
}

fn unpin(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "physics.unpin";
    let mut list = targets(s, p, CMD)?;
    let doc = &s.doc()?.doc;
    // No anchors chosen: every pin of the selected soft objects.
    let all = list.iter().all(|(_, pts)| pts.is_empty());
    if list.is_empty() {
        let st = s.doc()?;
        list = st.selection.objects.iter().filter_map(|id| soft_owner(doc, *id)).map(|o| (o, Vec::new())).collect();
    }
    let mut edits = Vec::new();
    for (owner, pts) in list {
        let Some(n) = doc.node(owner) else { continue };
        let (Some(mut st), Some(b)) = (soft_settings(n), bounds(doc, n)) else { continue };
        let Some(soft) = st.soft.as_mut() else { continue };
        let before = soft.pins.len();
        if all || pts.is_empty() {
            soft.pins.clear();
        } else {
            soft.pins.retain(|x| pts.iter().all(|q| (at(b, x) - *q).hypot() > UNPIN_REACH));
        }
        if soft.pins.len() != before {
            edits.push((owner, st));
        }
    }
    store(s, "Unpin Points", edits, Vec::new())
}

/// Where pins show on the canvas, with whether each is attached: on the session's bodies where
/// they are now while one is open, else on the selected soft objects at rest.
pub fn markers(s: &Session) -> Vec<(Point, bool)> {
    let mut out = Vec::new();
    if let Some(l) = live(s) {
        let world = l.sim.world();
        for (k, (_, pose, _)) in l.poses().iter().enumerate() {
            let Some(b) = world.body(k) else { continue };
            let particles: Option<&Vec<Point>> = match pose {
                vectorcraft_physics::pose::Pose::Soft(p) => Some(p),
                _ => None,
            };
            for (i, to) in &b.pins {
                let rest = b.mesh.as_ref().and_then(|m| m.rest.get(*i as usize)).copied();
                if let Some(q) = particles.and_then(|p| p.get(*i as usize).copied()).or(rest) {
                    out.push((q, to.is_some()));
                }
            }
        }
        return out;
    }
    let Some(st) = s.active() else { return out };
    let mut owners: Vec<NodeId> = st.selection.objects.iter().take(64).filter_map(|id| soft_owner(&st.doc, *id)).collect();
    owners.sort_unstable();
    owners.dedup();
    for owner in owners {
        let Some(n) = st.doc.node(owner) else { continue };
        let (Some(set), Some(b)) = (soft_settings(n), bounds(&st.doc, n)) else { continue };
        out.extend(set.soft.iter().flat_map(|x| x.pins.iter()).map(|pin| (at(b, pin), pin.attach_to.is_some())));
    }
    out
}
