//! PICABIA: Apply: the poses a physics session shows, baked into the art as one undo step
//! (PIC31.3).
//!
//! A rigid body's object moves and turns with the transform commands' own path (`transform_node`,
//! as phase 1's Simulate and Transform Each do), so live shapes stay live and pattern fills follow
//! General › Transform Pattern Tiles (or the `patterns` param). A soft body's object is replaced by
//! its warped art (`vectorcraft_physics::pose::pose_node`): its fills, strokes and gradients kept,
//! a bent live rectangle or ellipse a plain path.

use serde_json::Value;
use vectorcraft_doc::{NodeId, Scaling};
use vectorcraft_physics::mesh::SoftMesh;
use vectorcraft_physics::pose::{Pose, pose_node};

use crate::{Result, Session};

/// One body's pose to bake: the object, its pose, and its rest mesh for a soft body.
pub(crate) struct Baking<'a> {
    pub(crate) id: NodeId,
    pub(crate) pose: &'a Pose,
    pub(crate) mesh: Option<&'a SoftMesh>,
}

/// Bake `items` into the active document as one undo step labelled `label` (none when nothing
/// moved). Returns the objects changed. `p` may hold `patterns` (Transform Pattern Tiles). An
/// object listed twice is baked once (its first item), layers are left alone, and an object inside
/// another listed one is left to move with it.
pub(crate) fn bake(s: &mut Session, label: &str, p: &Value, items: &[Baking]) -> Result<Vec<NodeId>> {
    let doc = &s.doc()?.doc;
    // Each object once, never a layer, and never inside another one baked (it moves with that
    // one): the first item for an object wins.
    let mut seen = std::collections::HashSet::new();
    let items: Vec<&Baking> = items.iter().filter(|b| seen.insert(b.id)).filter(|b| doc.node(b.id).is_some_and(|n| !n.is_layer())).collect();
    let paths = doc.index_paths(items.iter().map(|b| b.id));
    let items: Vec<&Baking> = items
        .into_iter()
        .filter(|b| {
            let Some(mine) = paths.get(&b.id) else { return false };
            !paths.iter().any(|(other, path)| *other != b.id && path.len() < mine.len() && mine.starts_with(path))
        })
        .collect();
    let mut rigid = Vec::new();
    let mut soft = Vec::new();
    for b in items {
        match (b.pose, b.mesh) {
            (Pose::Rigid(Some(xf)), _) => rigid.push((b.id, *xf)),
            (Pose::Soft(particles), Some(mesh)) => {
                // The art is warped from the document as it is (the session's rest pose).
                if let Some(n) = doc.node(b.id)
                    && let Some(posed) = pose_node(n, mesh, particles)
                    && posed != *n
                {
                    soft.push((b.id, posed));
                }
            }
            _ => {}
        }
    }
    if rigid.is_empty() && soft.is_empty() {
        return Ok(Vec::new());
    }
    let rigid_ids: Vec<NodeId> = rigid.iter().map(|(id, _)| *id).collect();
    let sc = Scaling { patterns: crate::cmd::object::transform_patterns(s, p, &rigid_ids)?, ..Scaling::default() };
    let changed: Vec<NodeId> = rigid_ids.iter().copied().chain(soft.iter().map(|(id, _)| *id)).collect();
    s.edit(label, |d, _| {
        let paths = d.index_paths(changed.iter().copied());
        for (id, xf) in &rigid {
            if let Some(n) = paths.get(id).and_then(|path| d.node_mut_at(path)) {
                crate::cmd::object::transform_node(n, *xf, sc, true);
            }
        }
        for (id, posed) in soft {
            if let Some(n) = paths.get(&id).and_then(|path| d.node_mut_at(path)) {
                *n = posed;
            }
        }
        Ok(())
    })?;
    Ok(changed)
}
