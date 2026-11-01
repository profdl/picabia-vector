//! PICABIA: modifier stack commands (`modifier.*`).
//!
//! An object's stack lives in its `picabia.modifiers` foreign attribute (see
//! `vectorcraft_modifiers`), so save and reopen, copy and paste, duplicate and undo carry it like
//! any of the object's attributes. Every editing command is one undo step; `ids` (or `id`) default
//! to the selection and name the objects. Layers and sublayers are refused: modifiers go on
//! objects. A modifier in a stack is picked with `modifier` (its id, e.g. `"m1"`) or `index`
//! (0 = the top row, which runs first).
//!
//! An object whose modifier data is damaged refuses edits (its data is kept) until
//! `modifier.remove_all` clears it.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_modifiers::{Modifier, Stack};

use super::{CommandSpec, cmd, has_doc, has_selection, ok, targets};
use crate::{EngineError, Result, Session};

/// The kinds `modifier.add` takes, for messages.
const KIND_LIST: &str = "linearArray|circularArray|gridArray|mirror|transformEach|randomize|wallpaper|copyToShape|stencil";

/// The modifier rows `modifier.copy` copied (each one modifier object), for `modifier.paste`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Copied {
    pub(crate) rows: Vec<Value>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "modifier.add",
            "Add Modifier",
            [],
            None,
            "{ids?|id?: the objects (default: the selection), kind: linearArray|circularArray|gridArray|mirror|transformEach|randomize|wallpaper|copyToShape|stencil, props?: {…the kind's params, camelCase: points, degrees, scale factors (1 = 100%); copyToShape: target (an object id; refused: the object itself, its groups or its contents, a top-level layer, or an object copying back onto it), mode (points|alongPath|populate), count, scale, align, seed; stencil (ends the stack: rows below it are ignored): its params as docs/picabia/modifiers.md lists them}, index?: where in the stack (default: the end, which runs last)} add a modifier to each object's stack, as one undo step → {modifier: the new id on the first object, added: [{id, modifier}]}",
            has_selection,
            add
        ),
        cmd!(
            "modifier.remove",
            "Remove Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id (e.g. \"m1\"), index?: 0 = the top row} remove one modifier from each object's stack, as one undo step",
            has_selection,
            remove
        ),
        cmd!(
            "modifier.move",
            "Move Modifier",
            [],
            None,
            "{ids?|id?, from: index, to: index} move a modifier up or down each object's stack, as one undo step",
            has_selection,
            move_modifier
        ),
        cmd!(
            "modifier.set",
            "Set Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, props: {…params to change, camelCase, as modifier.add takes them; enabled too; a stencil material preset fills its limits; <field>Expr: a per-copy expression on transformEach and randomize amounts (i, n, t, value; \"\" clears)}} change a modifier's params, as one undo step",
            has_selection,
            set
        ),
        cmd!(
            "modifier.toggle",
            "Toggle Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, enabled?: true|false (default: flip)} turn a modifier on or off, as one undo step",
            has_selection,
            toggle
        ),
        cmd!(
            "modifier.copy",
            "Copy Modifiers",
            [],
            None,
            "{id?: the object (default: the first selected), modifier?: id | index?: one row (without: the whole stack)} copy modifier rows for modifier.paste (not the objects' clipboard) → {copied}",
            has_doc,
            copy
        ),
        cmd!(
            "modifier.paste",
            "Paste Modifiers",
            [],
            None,
            "{ids?|id?, replace?: replace the stacks (default: add below the existing rows)} paste the rows modifier.copy copied onto each object, with fresh ids, as one undo step → {pasted}",
            has_selection,
            paste
        ),
        cmd!(
            query "modifier.list",
            "List Modifiers",
            [],
            None,
            "{id?: the object (default: the first selected)} → {id, modifiers: [{id, enabled, kind, label, …params}], instances: copies drawn (0 = drawn normally), warnings: [{modifier?, message}], error?: why the modifier data can't be read}",
            has_doc,
            list
        ),
        cmd!(
            "modifier.remove_all",
            "Remove All Modifiers",
            [],
            None,
            "{ids?|id?} remove the whole modifier stack from each object (damaged data too), as one undo step",
            has_selection,
            remove_all
        ),
        cmd!(
            "modifier.apply",
            "Apply Modifiers",
            [],
            None,
            "{ids?|id?, upTo?: index (inclusive): bake only the rows down to it; the rows after it move onto the new group and stay live} replace each object by a group of real objects, one per modifier copy (named `Name 1…n` when the object has a name), in its place, as one undo step; a stencil row stays live on the group; objects whose stack makes no copies are left alone → {ids: the groups (selected), applied: [{id, group, copies}]}",
            has_selection,
            apply::apply
        ),
    ]
}

/// The objects a command edits: `ids`/`id` or the selection; layers and sublayers are refused.
fn objects(cmd: &str, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = targets(s, p).map_err(|e| super::bad(cmd, e.to_string()))?;
    if ids.is_empty() {
        return Err(super::bad(cmd, "select an object or pass `ids`"));
    }
    let doc = &s.doc()?.doc;
    for id in &ids {
        let n = doc.node(*id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
        if n.is_layer() {
            return Err(super::bad(cmd, format!("{} is a layer; modifiers go on objects", id.0)));
        }
    }
    Ok(ids)
}

/// The one object a query reads: `id` or the first selected.
fn one_object(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let id = targets(s, p).map_err(|e| super::bad(cmd, e.to_string()))?.first().copied();
    let id = id.ok_or_else(|| super::bad(cmd, "select an object or pass `id`"))?;
    s.doc()?.doc.node(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    Ok(id)
}

/// Object `id`'s stack (empty without modifier data); damaged data is refused, not overwritten.
pub(super) fn read(cmd: &str, doc: &Document, id: NodeId) -> Result<Stack> {
    let n = doc.node(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    match vectorcraft_modifiers::node_source(n) {
        None => Ok(Stack::default()),
        Some(src) => vectorcraft_modifiers::parse(src).map_err(|e| super::bad(cmd, format!("object {}: {e} (modifier.remove_all clears it)", id.0))),
    }
}

/// Store `stack` on object `id` (an empty stack removes the data).
pub(super) fn write(cmd: &str, doc: &mut Document, id: NodeId, stack: &Stack) -> Result<()> {
    let n = doc.node_mut(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    let src = (!stack.is_empty()).then(|| vectorcraft_modifiers::to_json(stack));
    vectorcraft_modifiers::set_node_source(n, src);
    Ok(())
}

/// Read, change with `f`, and write back every object's stack, as one undo step `label`.
fn edit(cmd: &str, label: &str, s: &mut Session, p: &Value, mut f: impl FnMut(&mut Stack, NodeId) -> Result<()>) -> Result<()> {
    let ids = objects(cmd, s, p)?;
    s.edit(label, |doc, _| {
        for id in ids {
            let mut stack = read(cmd, doc, id)?;
            f(&mut stack, id)?;
            write(cmd, doc, id, &stack)?;
        }
        Ok(())
    })
}

/// A non-negative integer param `key`: `None` when absent or null.
pub(super) fn opt_index(cmd: &str, p: &Value, key: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            v.as_u64().map(|i| Some(super::index(i))).ok_or_else(|| super::bad(cmd, format!("`{key}` must be a non-negative integer, not {v}")))
        }
    }
}

fn req_index(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    opt_index(cmd, p, key)?.ok_or_else(|| super::bad(cmd, format!("`{key}` required (an index in the stack)")))
}

/// Which modifier `p` names (`modifier` id or `index`) in `stack`, on object `id`.
pub(super) fn pick(cmd: &str, p: &Value, stack: &Stack, id: NodeId) -> Result<usize> {
    let given = p.get("modifier").filter(|v| !v.is_null());
    let at = match (given, opt_index(cmd, p, "index")?) {
        (Some(m), None) => {
            let m = m.as_str().ok_or_else(|| super::bad(cmd, "`modifier` must be a modifier id such as \"m1\""))?;
            stack.position(m).ok_or_else(|| super::bad(cmd, format!("object {} has no modifier `{m}`", id.0)))?
        }
        (None, Some(i)) => i,
        (Some(_), Some(_)) => return Err(super::bad(cmd, "pass `modifier` or `index`, not both")),
        (None, None) => return Err(super::bad(cmd, "`modifier` (an id) or `index` required")),
    };
    if at >= stack.len() {
        return Err(super::bad(cmd, format!("object {} has no modifier at index {at} ({} in its stack)", id.0, stack.len())));
    }
    Ok(at)
}

pub(super) fn props_of<'a>(cmd: &str, p: &'a Value, required: bool) -> Result<Option<&'a Map<String, Value>>> {
    match p.get("props") {
        None | Some(Value::Null) if required => Err(super::bad(cmd, "`props` required")),
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(super::bad(cmd, "`props` must be an object")),
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.add";
    let kind = super::text_param(p, "kind", CMD)?.ok_or_else(|| super::bad(CMD, format!("`kind` required ({KIND_LIST})")))?;
    let mut template = Modifier::new("", kind).ok_or_else(|| super::bad(CMD, format!("unknown kind `{kind}` (one of {KIND_LIST})")))?;
    if let Some(props) = props_of(CMD, p, false)? {
        template.set_props(props).map_err(|e| super::bad(CMD, e.to_string()))?;
    }
    let at = opt_index(CMD, p, "index")?;
    let mut added = Vec::new();
    edit(CMD, "Add Modifier", s, p, |stack, id| {
        let m = stack.insert(at.unwrap_or(usize::MAX), template.clone()).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))?;
        added.push(json!({"id": id.0, "modifier": m.id}));
        Ok(())
    })?;
    let first = added.first().and_then(|a| a.get("modifier")).cloned().unwrap_or(Value::Null);
    Ok(json!({"modifier": first, "added": added}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.remove";
    edit(CMD, "Remove Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        if at < stack.modifiers.len() {
            stack.modifiers.remove(at);
        }
        Ok(())
    })?;
    ok()
}

fn move_modifier(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.move";
    let (from, to) = (req_index(CMD, p, "from")?, req_index(CMD, p, "to")?);
    edit(CMD, "Move Modifier", s, p, |stack, id| {
        let n = stack.len();
        if from >= n || to >= n {
            return Err(super::bad(CMD, format!("object {} has {n} modifiers (indexes 0..{})", id.0, n.saturating_sub(1))));
        }
        let m = stack.modifiers.remove(from);
        stack.modifiers.insert(to, m);
        Ok(())
    })?;
    ok()
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.set";
    let props = props_of(CMD, p, true)?.cloned().unwrap_or_default();
    edit(CMD, "Set Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        let m = stack.modifiers.get_mut(at).ok_or_else(|| super::bad(CMD, "no such modifier"))?;
        m.set_props(&props).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))
    })?;
    ok()
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.toggle";
    let enabled = match p.get("enabled") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| super::bad(CMD, "`enabled` must be true or false"))?),
    };
    edit(CMD, "Toggle Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        let m = stack.modifiers.get_mut(at).ok_or_else(|| super::bad(CMD, "no such modifier"))?;
        m.enabled = enabled.unwrap_or(!m.enabled);
        Ok(())
    })?;
    ok()
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.copy";
    let id = one_object(CMD, s, p)?;
    let stack = read(CMD, &s.doc()?.doc, id)?;
    let picked = p.get("modifier").is_some_and(|v| !v.is_null()) || p.get("index").is_some_and(|v| !v.is_null());
    let rows: Vec<Value> = if picked {
        let at = pick(CMD, p, &stack, id)?;
        stack.modifiers.get(at).map(Modifier::to_value).into_iter().collect()
    } else {
        stack.modifiers.iter().map(Modifier::to_value).collect()
    };
    if rows.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), format!("object {} has no modifiers", id.0)));
    }
    let copied = rows.len();
    s.picabia_modifiers.rows = rows;
    Ok(json!({"copied": copied}))
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.paste";
    if s.picabia_modifiers.rows.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "no copied modifiers".into()));
    }
    let rows = s.picabia_modifiers.rows.iter().map(Modifier::from_value).collect::<std::result::Result<Vec<_>, _>>();
    let rows = rows.map_err(|e| super::bad(CMD, e.to_string()))?;
    let replace = super::switch_or(p, "replace", false, CMD)?;
    edit(CMD, "Paste Modifiers", s, p, |stack, id| {
        if replace {
            stack.modifiers.clear();
        }
        for m in &rows {
            stack.insert(usize::MAX, m.clone()).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))?;
        }
        Ok(())
    })?;
    Ok(json!({"pasted": rows.len()}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.list";
    let id = one_object(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let node = doc.node(id).ok_or_else(|| super::bad(CMD, format!("no object with id {}", id.0)))?;
    let mut out = json!({"id": id.0, "modifiers": [], "instances": 0, "warnings": []});
    match vectorcraft_modifiers::node_stack(node) {
        None => {}
        Some(Err(e)) => out["error"] = json!(e.to_string()),
        Some(Ok(stack)) => {
            let rows: Vec<Value> = stack
                .modifiers
                .iter()
                .map(|m| {
                    let mut v = m.to_value();
                    if let Some(o) = v.as_object_mut() {
                        o.insert("label".into(), json!(m.label()));
                    }
                    v
                })
                .collect();
            out["modifiers"] = json!(rows);
            let targets = vectorcraft_modifiers::DocTargets::new(doc, id);
            let o = vectorcraft_modifiers::evaluate_with(&stack, vectorcraft_modifiers::node_bounds(node), &targets);
            out["instances"] = json!(o.instances.len());
            out["warnings"] = json!(o.warnings.iter().map(|w| json!({"modifier": w.modifier, "message": w.message})).collect::<Vec<_>>());
        }
    }
    Ok(out)
}

fn remove_all(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.remove_all";
    let ids = objects(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let with: Vec<NodeId> = ids.into_iter().filter(|id| doc.node(*id).is_some_and(vectorcraft_modifiers::has_modifiers)).collect();
    // Nothing to remove: no undo step.
    if with.is_empty() {
        return ok();
    }
    s.edit("Remove All Modifiers", |doc, _| {
        for id in with {
            if let Some(n) = doc.node_mut(id) {
                vectorcraft_modifiers::set_node_source(n, None);
            }
        }
        Ok(())
    })?;
    ok()
}

mod apply;
#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod tests;
