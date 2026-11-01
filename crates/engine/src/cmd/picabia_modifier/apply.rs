//! PICABIA: `modifier.apply`: bake an object's modifier copies into real objects.
//!
//! Each source object is replaced, in the same parent and at the same place in the stacking
//! order, by a group with its name holding one real object per copy (named `Name 1…n` when the
//! source has a name). Each copy is a deep copy of the source with fresh ids
//! ([`Document::reid`], as Duplicate makes them) whose geometry is transformed by the copy's
//! instance transform, the way the Transform commands transform objects
//! (`cmd::object::transform_node`), and whose stack is taken off. The groups end up selected.
//!
//! The copy's transform scales its strokes, effects, live corners and pattern tiles with it, so
//! the baked copy looks like the copy drawn live, whatever Preferences › Scale Strokes & Effects
//! says.
//!
//! `upTo` (an index, inclusive) bakes only `stack[0..=upTo]`; the rows after it move onto the new
//! group and keep running live (what was baked is now one object, and the rest of the recipe runs
//! on it). Array rows repeat the whole previous set as a block anyway, so they repeat the group
//! exactly as they repeated the copies (a mirror's axis, placed relative to the pivot, is shifted
//! from the group's centre back to the source's pivot). Still, the group's pivot is its own
//! centre, so what turns or scales *about* the pivot moves differently: an aligned circular array
//! turns each block about the group's centre, a linear array's rotate/scale each pivots there, and
//! per-copy rows (transform each, randomize, wallpaper) now act on the whole block, as a modifier
//! on any group does.
//!
//! A stencil row is never baked here: `upTo` stops above it, and it moves onto the group with the
//! rows after it (it cuts the same copies there).
//!
//! Geometry rows (`resample`, `trim`, `tidy`, `deform`, `displace`, `screen`) are baked too: the copies get the reshaped paths the
//! renderer draws (`vectorcraft_modifiers::Shaper`), so a stack of geometry rows alone applies as a
//! group of one reshaped copy.
//!
//! An object whose stack makes no copies (empty, all rows off, or one unmoved copy and no geometry
//! row) is left alone;
//! when no object has anything to apply the command is refused. An object that can't be replaced
//! by a group where it is (a member of a compound path, blend, envelope, repeat or intertwine, or
//! a clipping path) is refused.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Vec2};
use vectorcraft_modifiers::{MirrorAxis, ModifierKind, Stack};

use super::{objects, opt_index, read};
use crate::{EngineError, Result, Session};

const CMD: &str = "modifier.apply";

/// The most objects one `modifier.apply` creates (copies × objects per copy), checked before
/// copying.
pub const MAX_APPLY_OBJECTS: usize = 100_000;

/// What applying one object's stack makes.
struct Plan {
    id: NodeId,
    /// The instance transforms of the baked rows, one copy each.
    copies: Vec<Affine>,
    /// The baked rows' evaluation: its geometry rows reshape the copies as they are drawn.
    out: vectorcraft_modifiers::Output,
    /// The rows that stay live, moved onto the group.
    rest: Stack,
    /// The source's pivot (the centre of its geometric bounds).
    pivot: vectorcraft_geom::Point,
}

pub(super) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let up_to = opt_index(CMD, p, "upTo")?;
    let ids = objects(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let mut seen = std::collections::BTreeSet::new();
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| seen.insert(*id)).collect();
    let ids = super::super::edit::outermost(doc, ids);
    let mut budget = MAX_APPLY_OBJECTS;
    let mut plans = Vec::new();
    for id in ids {
        if let Some(plan) = plan(doc, id, up_to, &mut budget)? {
            plans.push(plan);
        }
    }
    if plans.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "no modifier copies to apply (no active modifiers)".into()));
    }
    let applied = s.edit("Apply Modifiers", |d, sel| {
        let mut applied = Vec::with_capacity(plans.len());
        let mut groups = Vec::with_capacity(plans.len());
        let mut copy_ids = Vec::new();
        let mut live = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for plan in plans {
            let (id, copies) = (plan.id, plan.copies.len());
            let (group, ids, notes) = bake(d, &plan)?;
            // What a geometry row couldn't do (skipped over the anchor cap…) is said, not lost.
            for n in notes {
                let w = format!("object {}: {n}", id.0);
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            applied.push((id, group.id, copies));
            if !plan.rest.is_empty() {
                live.push((group.id, plan.rest, plan.pivot));
            }
            copy_ids.extend(ids);
            groups.push((id, vec![group]));
        }
        // Every source is replaced in one pass over the tree.
        d.insert_beside(groups, true)?;
        // Copied type whose layout caches `reid` dropped has them again.
        super::super::typecmd::refresh_bounds_of(d, &copy_ids);
        // The rows after `upTo` keep running on the group.
        for (gid, rest, pivot) in live {
            let g = d.node(gid).ok_or_else(|| super::super::bad(CMD, "the new group is gone"))?;
            let rest = repivot(rest, pivot - vectorcraft_modifiers::node_bounds_in_doc(d, g).center());
            let g = d.node_mut(gid).ok_or_else(|| super::super::bad(CMD, "the new group is gone"))?;
            vectorcraft_modifiers::set_node_source(g, Some(vectorcraft_modifiers::to_json(&rest)));
        }
        sel.set(applied.iter().map(|a| a.1));
        Ok((applied, warnings))
    })?;
    let (applied, warnings) = applied;
    let groups: Vec<u64> = applied.iter().map(|a| a.1.0).collect();
    let rows: Vec<Value> = applied.iter().map(|(id, g, n)| json!({"id": id.0, "group": g.0, "copies": n})).collect();
    Ok(json!({"ids": groups, "applied": rows, "warnings": warnings}))
}

/// What applying object `id` makes, or `None` when its stack makes no copies.
fn plan(doc: &Document, id: NodeId, up_to: Option<usize>, budget: &mut usize) -> Result<Option<Plan>> {
    let stack = read(CMD, doc, id)?;
    let Some(last) = stack.len().checked_sub(1) else { return Ok(None) };
    let cut = up_to.unwrap_or(last);
    if cut > last {
        return Err(super::super::bad(CMD, format!("object {} has {} modifiers: `upTo` is at most {last}", id.0, stack.len())));
    }
    // A stencil row ends the stack and has no copies to bake: only the rows above it are baked,
    // and it stays live on the group (with any rows below it), cutting the same copies.
    let cut = match stack.modifiers.iter().position(|m| m.enabled && matches!(m.kind, ModifierKind::Stencil(_))) {
        Some(0) => return Ok(None),
        Some(at) => cut.min(at - 1),
        None => cut,
    };
    let baked = Stack { modifiers: stack.modifiers.get(..=cut).map(<[_]>::to_vec).unwrap_or_default(), ..stack.clone() };
    let rest = Stack { modifiers: stack.modifiers.get(cut + 1..).map(<[_]>::to_vec).unwrap_or_default(), ..stack };
    let node = doc.node(id).ok_or_else(|| super::super::bad(CMD, format!("no object with id {}", id.0)))?;
    // The pivot the renderer uses (a symbol instance pivots on its art).
    let bounds = vectorcraft_modifiers::node_bounds_in_doc(doc, node);
    let out = vectorcraft_modifiers::evaluate_with(&baked, bounds, &vectorcraft_modifiers::DocTargets::new(doc, id));
    // One unmoved copy is the object itself, unless geometry rows reshape it.
    let unmoved = matches!(out.instances.as_slice(), [one] if one.transform == Affine::IDENTITY) && !out.has_geometry();
    if out.instances.is_empty() || unmoved {
        return Ok(None);
    }
    replaceable(doc, node)?;
    let mut per_copy = 0usize;
    node.walk(&mut |_| per_copy += 1);
    let wanted = per_copy.max(1).saturating_mul(out.instances.len());
    if wanted > *budget {
        return Err(super::super::bad(CMD, format!("applying would make {wanted} objects (at most {MAX_APPLY_OBJECTS} at once)")));
    }
    *budget -= wanted;
    Ok(Some(Plan { id, copies: out.instances.iter().map(|i| i.transform).collect(), out, rest, pivot: bounds.center() }))
}

/// Refuse `node` when a group can't take its place.
fn replaceable(doc: &Document, node: &Node) -> Result<()> {
    let name = node.display_name();
    let parent = doc.parent_of(node.id).and_then(|p| doc.node(p));
    // The first member of a clip group or of a layer with a clipping mask clips the rest, whatever
    // kind it is (a path, a compound path, type…): a group can't take its place.
    let clips = parent.is_some_and(|p| p.clips() && p.children().and_then(|c| c.first()).is_some_and(|c| c.id == node.id));
    if clips || matches!(node.kind, NodeKind::Path { clipping: true, .. }) {
        return Err(super::super::bad(CMD, format!("\"{name}\" is a clipping path: release the clipping mask first")));
    }
    match parent.map(|p| &p.kind) {
        Some(NodeKind::Layer { .. } | NodeKind::Group { .. }) if !parent.is_some_and(Node::is_compound_shape) => Ok(()),
        Some(_) => Err(super::super::bad(CMD, format!("\"{name}\" is part of another object: release or expand it first"))),
        None => Err(super::super::bad(CMD, format!("\"{name}\" is a layer; modifiers go on objects"))),
    }
}

/// The group of `plan.id`'s copies (not in the tree yet), the copies' ids, and what the geometry
/// rows said (a row skipped over the anchor cap: its reshape isn't in the copies).
fn bake(d: &mut Document, plan: &Plan) -> Result<(Node, Vec<NodeId>, Vec<String>)> {
    let mut node = d.node(plan.id).cloned().ok_or_else(|| super::super::bad(CMD, format!("no object with id {}", plan.id.0)))?;
    // The object's transparency and its own attributes go on the group, as the renderer draws
    // the copies (composited together, once): the copies are the object without them.
    let gid = d.alloc_id();
    let mut group = Node::group(gid, vec![]);
    group.name = node.name.clone();
    group.opacity = std::mem::replace(&mut node.opacity, 1.0);
    group.blend = std::mem::take(&mut node.blend);
    group.isolate = std::mem::take(&mut node.isolate);
    group.extras_mut().mask = node.extras_mut().mask.take();
    group.extras_mut().attrs = node.extras_mut().attrs.take();
    vectorcraft_modifiers::set_node_source(&mut group, None);
    let sc = vectorcraft_render::picabia::scaling();
    // Geometry rows reshape the object before copying and each copy after its transform, as the
    // renderer draws them (the tree's shape is kept, so targets still pair up below).
    let mut shaper = vectorcraft_modifiers::Shaper::new(&plan.out);
    let mut shaped = node.clone();
    shaper.source(&mut shaped);
    let mut kids = Vec::with_capacity(plan.copies.len());
    for (k, t) in plan.copies.iter().enumerate() {
        let mut c = d.reid(&shaped);
        // Targets inside the object point at the ones inside this copy.
        super::targets::remap_copy(&node, &mut c);
        if *t != Affine::IDENTITY {
            super::super::object::transform_node(&mut c, *t, sc, true);
        }
        shaper.copy_at(&mut c, k);
        vectorcraft_modifiers::set_node_source(&mut c, None);
        if let Some(name) = &node.name {
            c.name = Some(format!("{name} {}", k + 1));
        }
        kids.push(Arc::new(c));
    }
    let copy_ids: Vec<NodeId> = kids.iter().map(|c| c.id).collect();
    if let Some(ch) = group.children_mut() {
        *ch = kids;
    }
    Ok((group, copy_ids, shaper.notes))
}

/// Shift the pivot-relative params of `stack` by `d` (the old pivot minus the new one), so its
/// rows place copies as they did around the old pivot. Only a mirror's axis can follow: the other
/// kinds either don't depend on the pivot (translations) or turn about it, which no param moves.
pub(super) fn repivot(mut stack: Stack, d: Vec2) -> Stack {
    if !(d.x.is_finite() && d.y.is_finite()) {
        return stack;
    }
    for m in &mut stack.modifiers {
        if let ModifierKind::Mirror(mirror) = &mut m.kind {
            mirror.offset += match mirror.axis {
                MirrorAxis::Vertical | MirrorAxis::Both => d.x,
                MirrorAxis::Horizontal => d.y,
            };
        }
    }
    stack
}
