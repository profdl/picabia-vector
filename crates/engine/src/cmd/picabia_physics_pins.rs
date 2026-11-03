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
            "{id?: the soft object, points?: [[x, y] | {x, y}] (document space; default: the anchors selected with Direct Selection on soft objects), attachTo?: an object id (it must have physics settings and not be soft: the points move with it), attachToSelected?: attach to the other selected object instead} pin points of soft bodies, as one undo step ('Pin Points'); at most 64 pins an object → {pins: [{id, count}]}",
            has_doc,
            pin
        ),
        cmd!(
            "physics.unpin",
            "Unpin Points",
            [],
            None,
            "{id?, points?: [[x, y] | {x, y}] (pins within 4 pt of each go; default: the anchors selected with Direct Selection, or every pin of the selected soft objects when no anchor is selected)} remove pins, as one undo step ('Unpin Points') → {pins: [{id, count}]}",
            has_doc,
            unpin
        ),
    ]
}

/// The soft body holding object `id`: itself or the nearest container with soft settings.
fn soft_owner(doc: &Document, id: NodeId) -> Option<NodeId> {
    let paths = doc.index_paths([id]);
    let path = paths.get(&id)?;
    (1..=path.len()).rev().filter_map(|k| path.get(..k)).filter_map(|p| doc.node_at(p)).find(|n| soft_settings(n).is_some()).map(|n| n.id)
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
fn store(s: &mut Session, label: &str, edits: Vec<(NodeId, Settings)>) -> Result<Value> {
    let summary: Vec<Value> = edits.iter().map(|(id, st)| json!({"id": id.0, "count": st.soft.as_ref().map_or(0, |x| x.pins.len())})).collect();
    if edits.is_empty() {
        return Ok(json!({"pins": summary}));
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
    Ok(json!({"pins": summary}))
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
            let owners: Vec<NodeId> = list.iter().map(|(o, _)| *o).collect();
            let st = s.doc()?;
            let other =
                st.selection.objects.iter().copied().find(|id| !owners.contains(id) && soft_owner(doc, *id).is_none_or(|o| !owners.contains(&o)));
            Some(other.ok_or_else(|| bad(CMD, "select the object to attach to as well"))?)
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
    for (owner, pts) in list.into_iter().filter(|(_, pts)| !pts.is_empty()) {
        if Some(owner) == attach {
            return Err(bad(CMD, "points can't be attached to their own object"));
        }
        let Some(n) = doc.node(owner) else { continue };
        let (Some(mut st), Some(b)) = (soft_settings(n), n.geometric_bounds()) else { continue };
        let soft = st.soft.get_or_insert_with(Default::default);
        for q in pts {
            let (u, v) = place(b, q);
            if let Some(same) = soft.pins.iter_mut().find(|x| (x.u - u).abs() < 1e-9 && (x.v - v).abs() < 1e-9) {
                same.attach_to = attach.map(|a| a.0);
            } else if soft.pins.len() < MAX_PINS {
                soft.pins.push(Pin::new(u, v, attach.map(|a| a.0)));
            }
        }
        edits.push((owner, st));
    }
    store(s, "Pin Points", edits)
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
        let (Some(mut st), Some(b)) = (soft_settings(n), n.geometric_bounds()) else { continue };
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
    store(s, "Unpin Points", edits)
}

/// Where pins show on the canvas, with whether each is attached: on the session's bodies where
/// they are now while one is open, else on the selected soft objects at rest.
pub fn markers(s: &Session) -> Vec<(Point, bool)> {
    let mut out = Vec::new();
    if let Some(l) = live(s) {
        for ((_, pose, _), b) in l.poses().iter().zip(&l.bodies) {
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
    for id in st.selection.objects.iter().take(64) {
        let Some(owner) = soft_owner(&st.doc, *id) else { continue };
        let Some(n) = st.doc.node(owner) else { continue };
        let (Some(set), Some(b)) = (soft_settings(n), n.geometric_bounds()) else { continue };
        out.extend(set.soft.iter().flat_map(|x| x.pins.iter()).map(|pin| (at(b, pin), pin.attach_to.is_some())));
    }
    out
}
