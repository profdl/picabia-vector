//! PICABIA: copy-to-shape targets in the `modifier.*` commands, `modifier.duplicate`, and target
//! ids following copies of objects. A screen row's image (its `image`, when its tone comes from
//! one) is a target too: checked, remapped and cleared alike (`vectorcraft_modifiers::Stack::targets`).
//!
//! Every stack edit ([`super::edit`]) checks the copy-to-shape targets it sets or re-enables
//! ([`check_changed`]): the object itself, one of its groups or its contents, a top-level layer,
//! a missing object or a loop back through other targets is refused
//! (`vectorcraft_modifiers::check_target`). Targets already in a stack aren't checked again, so a
//! deleted target doesn't block unrelated edits (it places no copies, with a warning).
//! `modifier.targets` lists the objects an object can copy onto, for the panel's target picker.
//!
//! Copying objects (duplicate, copying moves and transforms, paste, `modifier.apply`) gives the
//! copies new ids; [`remap_copies`] and [`remap_pasted`] then point a copied source at its copied
//! target when both were copied together, by walking each original and its copy side by side.
//! A target left behind keeps pointing at the original, except in a paste from another document,
//! where it names nothing here: it is cleared (no target chosen).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_modifiers::{ModifierKind, Stack};

use super::{objects, pick, read, write};
use crate::{Result, Session};

/// Stacks longer than this aren't rewritten (`vectorcraft_modifiers::parse` refuses them anyway).
const MAX_REWRITE_BYTES: usize = vectorcraft_modifiers::MAX_SOURCE_BYTES;

/// The enabled copy-to-shape `(modifier id, target)` pairs of `stack`.
fn pairs(stack: &Stack) -> BTreeSet<(String, String)> {
    stack.targets().map(|(m, t)| (m.to_string(), t.to_string())).collect()
}

/// Refuse the copy-to-shape targets `after` sets on object `id` that `before` didn't have.
pub(super) fn check_changed(cmd: &str, doc: &Document, id: NodeId, before: &Stack, after: &Stack) -> Result<()> {
    let old = pairs(before);
    for (m, t) in pairs(after).difference(&old) {
        let why = |e: String| super::super::bad(cmd, format!("object {}, modifier {m}: {e}", id.0));
        let target = vectorcraft_modifiers::parse_target(t).map_err(why)?;
        vectorcraft_modifiers::check_target(doc, id, target).map_err(why)?;
    }
    Ok(())
}

/// What the panel's target picker shows for object `source`: the objects it can copy onto, as
/// `(id, name)` in layer order (see `vectorcraft_modifiers::valid_targets`).
pub fn target_choices(doc: &Document, source: NodeId) -> Vec<(NodeId, String)> {
    vectorcraft_modifiers::valid_targets(doc, source).into_iter().filter_map(|t| Some((t, doc.node(t)?.display_name()))).collect()
}

/// What the panel's image picker shows for a screen row on object `source`: the image objects it
/// can read its tone from, as `(id, name)` in layer order.
pub fn image_choices(doc: &Document, source: NodeId) -> Vec<(NodeId, String)> {
    let mut ids = Vec::new();
    doc.walk(|n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)) && ids.len() < vectorcraft_modifiers::MAX_LISTED_TARGETS {
            ids.push(n.id);
        }
    });
    ids.into_iter()
        .filter(|t| vectorcraft_modifiers::check_target(doc, source, *t).is_ok())
        .filter_map(|t| Some((t, doc.node(t)?.display_name())))
        .collect()
}

/// Whether object `source` can copy onto `target`; `Err` is the reason (for the picker's tooltip
/// or the panel's warning).
pub fn check_target(doc: &Document, source: NodeId, target: NodeId) -> std::result::Result<(), String> {
    vectorcraft_modifiers::check_target(doc, source, target)
}

/// `modifier.targets {id?, modifier? | index?}`: the objects `id` (default: the first selected)
/// can copy onto, and, for a picked copy-to-shape row, its current target and whether it is valid.
pub(super) fn list_targets(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.targets";
    let id = super::one_object(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let rows: Vec<Value> = target_choices(doc, id)
        .into_iter()
        .map(|(t, name)| json!({"id": t.0, "name": name, "kind": doc.node(t).map(|n| n.kind_label()).unwrap_or_default()}))
        .collect();
    let mut out = json!({"id": id.0, "targets": rows});
    if p.get("modifier").is_some_and(|v| !v.is_null()) || p.get("index").is_some_and(|v| !v.is_null()) {
        let stack = read(CMD, doc, id)?;
        let at = pick(CMD, p, &stack, id)?;
        if let Some(ModifierKind::CopyToShape(c)) = stack.modifiers.get(at).map(|m| &m.kind) {
            out["target"] = json!(c.target);
            let check = vectorcraft_modifiers::parse_target(&c.target).and_then(|t| check_target(doc, id, t));
            out["valid"] = json!(check.is_ok());
            if let Err(e) = check {
                out["reason"] = json!(e);
            }
        }
    }
    Ok(out)
}

/// `modifier.duplicate {ids?, modifier? | index?}`: a copy of the row right below it, with a fresh
/// id, as one undo step.
pub(super) fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.duplicate";
    let ids = objects(CMD, s, p)?;
    let made = s.edit("Duplicate Modifier", |doc, _| {
        let mut made = Vec::with_capacity(ids.len());
        for id in ids {
            let mut stack = read(CMD, doc, id)?;
            let at = pick(CMD, p, &stack, id)?;
            let Some(mut copy) = stack.modifiers.get(at).cloned() else { continue };
            copy.id = String::new();
            let m = stack.insert(at.saturating_add(1), copy).map_err(|e| super::super::bad(CMD, format!("object {}: {e}", id.0)))?;
            made.push(json!({"id": id.0, "modifier": m.id}));
            write(CMD, doc, id, &stack)?;
        }
        Ok(made)
    })?;
    let first = made.first().and_then(|a| a.get("modifier")).cloned().unwrap_or(Value::Null);
    Ok(json!({"modifier": first, "duplicated": made}))
}

/// Add to `map` each id of `orig`'s subtree with the id of the node in the same place in `copy`.
/// Where the two trees differ in shape the walk stops (those ids aren't mapped).
fn id_map(orig: &Node, copy: &Node, map: &mut BTreeMap<NodeId, NodeId>, depth: usize) {
    map.insert(orig.id, copy.id);
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    let (Some(a), Some(b)) = (orig.children(), copy.children()) else { return };
    if a.len() != b.len() {
        return;
    }
    for (a, b) in a.iter().zip(b) {
        id_map(a, b, map, depth + 1);
    }
}

/// The kinds that name another object, and the field that names it.
const TARGET_FIELDS: &[(&str, &str)] = &[("copyToShape", "target"), ("screen", "image")];

/// How each kind in [`TARGET_FIELDS`] appears in a stack's text: its quoted id.
const TARGET_KINDS: &[&str] = &["\"copyToShape\"", "\"screen\""];

/// Whether stack text `src` may name another object.
fn may_name(src: &str) -> bool {
    TARGET_KINDS.iter().any(|kind| src.contains(kind))
}

/// Whether `n` has a stack that may hold copy-to-shape targets (or a screen's image).
fn may_target(n: &Node) -> bool {
    vectorcraft_modifiers::node_source(n).is_some_and(may_name)
}

/// Whether `n`'s subtree has a node [`may_target`].
fn subtree_may_target(n: &Node) -> bool {
    let mut any = false;
    n.walk(&mut |c| any |= may_target(c));
    any
}

/// `src` with the copy-to-shape targets that `map` covers pointing at their copies, and (unless
/// `keep_outside`) the others cleared; `None` when nothing changes or the text isn't a stack.
fn rewrite_source(src: &str, map: &BTreeMap<NodeId, NodeId>, keep_outside: bool) -> Option<String> {
    if src.len() > MAX_REWRITE_BYTES || !may_name(src) {
        return None;
    }
    let mut v = serde_json::from_str::<Value>(src).ok()?;
    let mut changed = false;
    for m in v.get_mut("stack").and_then(Value::as_array_mut).into_iter().flatten() {
        let Some(o) = m.as_object_mut() else { continue };
        let kind = o.get("kind").and_then(Value::as_str);
        let Some(&(_, field)) = TARGET_FIELDS.iter().find(|(k, _)| Some(*k) == kind) else { continue };
        let old = match o.get(field) {
            Some(Value::String(s)) if !s.trim().is_empty() => vectorcraft_modifiers::parse_target(s).ok(),
            Some(Value::Number(x)) => x.as_u64().map(NodeId),
            _ => continue,
        };
        match old.and_then(|t| map.get(&t)) {
            Some(new) => {
                o.insert(field.into(), Value::String(new.0.to_string()));
                changed = true;
            }
            None if !keep_outside => {
                o.insert(field.into(), Value::String(String::new()));
                changed = true;
            }
            None => {}
        }
    }
    if !changed {
        return None;
    }
    serde_json::to_string(&v).ok()
}

/// [`rewrite_source`] on every stack in `n`'s subtree (only the branches that have one are
/// copied on write).
fn rewrite(n: &mut Node, map: &BTreeMap<NodeId, NodeId>, keep_outside: bool, depth: usize) {
    if let Some(new) = vectorcraft_modifiers::node_source(n).and_then(|src| rewrite_source(src, map, keep_outside)) {
        vectorcraft_modifiers::set_node_source(n, Some(new));
    }
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        if subtree_may_target(c) {
            rewrite(Arc::make_mut(c), map, keep_outside, depth + 1);
        }
    }
}

/// Point the copy-to-shape targets inside `copy` (a fresh copy of `orig`, not in a document yet)
/// that point inside `orig` at their copies (`modifier.apply`'s copies).
pub(super) fn remap_copy(orig: &Node, copy: &mut Node) {
    if !subtree_may_target(copy) {
        return;
    }
    let mut map = BTreeMap::new();
    id_map(orig, copy, &mut map, 0);
    rewrite(copy, &map, true, 0);
}

/// Called by `copy_above` before it copies `roots`: the originals it will copy, as they are now,
/// in order (the roots in the document; a root inside another is copied after the outer one
/// changed). Empty, and nothing cloned, when none of them holds a stack that may copy onto
/// another object (the common case: the copy then needs no remapping).
pub(crate) fn snapshot(d: &Document, roots: &[NodeId]) -> Vec<Node> {
    if !roots.iter().any(|r| d.node(*r).is_some_and(subtree_may_target)) {
        return Vec::new();
    }
    roots.iter().filter_map(|r| d.node(*r).cloned()).collect()
}

/// Called by `copy_above` after copying: `copies` are the copies of `originals` ([`snapshot`]), in
/// order. Targets copied together with their source point at the copies.
pub(crate) fn remap_copies(d: &mut Document, originals: Vec<Node>, copies: &[NodeId]) {
    if originals.is_empty() {
        return;
    }
    let pairs: Vec<(Node, NodeId)> = originals.into_iter().zip(copies.iter().copied()).collect();
    remap(d, &pairs, true);
}

/// Called by paste: `pasted` holds a copy of each of `originals` (the clipboard's objects) per
/// placement, in order. Targets pasted together with their source point at the pasted copies;
/// from another document (`same_doc` false), the targets left behind are cleared.
pub(crate) fn remap_pasted(d: &mut Document, originals: &[Node], pasted: &[NodeId], same_doc: bool) {
    if originals.is_empty() || !originals.iter().any(subtree_may_target) {
        return;
    }
    for chunk in pasted.chunks(originals.len()) {
        let pairs: Vec<(Node, NodeId)> = originals.iter().cloned().zip(chunk.iter().copied()).collect();
        remap(d, &pairs, same_doc);
    }
}

/// Rewrite the copies' targets through the id map of all `(original, copy id)` pairs together.
fn remap(d: &mut Document, pairs: &[(Node, NodeId)], keep_outside: bool) {
    if !pairs.iter().any(|(o, _)| subtree_may_target(o)) {
        return;
    }
    let mut map = BTreeMap::new();
    for (orig, copy) in pairs {
        if let Some(c) = d.node(*copy) {
            id_map(orig, c, &mut map, 0);
        }
    }
    for (orig, copy) in pairs {
        if d.node(*copy).is_some_and(subtree_may_target)
            && let Some(c) = d.node_mut(*copy)
        {
            // Inside one copy, a target copied along in the same copy wins over a copy of it made
            // as another root (Duplicate of a group and of an object inside it).
            let mut own = BTreeMap::new();
            id_map(orig, c, &mut own, 0);
            let mut combined = map.clone();
            combined.extend(own);
            rewrite(c, &combined, keep_outside, 0);
        }
    }
}
