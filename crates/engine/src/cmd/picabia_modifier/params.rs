//! PICABIA: parameters (`param.*`) and field bindings (`modifier.bind`, `modifier.unbind`).
//!
//! Parameters are the document's (`vectorcraft_modifiers::params`, kept in its plug-in data), so
//! every edit is one undo step and they save, reopen and copy with the document. A modifier row's
//! number field binds to a parameter or an expression (`vectorcraft_modifiers::bind`); the binding
//! lives in the row, so it follows the row and goes with it. See `docs/picabia/parameters.md`.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_modifiers::{Binding, Parameter, Parameters, Scope};

use super::{edit, pick};
use crate::cmd::{CommandSpec, bad, cmd, has_doc, has_selection, text_param};
use crate::{EngineError, Result, Session};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "param.add",
            "Add Parameter",
            [],
            None,
            "{name: a letter or _ followed by letters or digits or _ (at most 32; not i, n, t, value, pi or a function; unique ignoring case), type?: number (default) | text, value?, min?, max? (number: the range, default 0–100 widened to take the value), step? (the slider's step, default 1; 0 = any), source?: a VectorCraft variable whose value in the active data set is the parameter's} add a document parameter (at most 256), as one undo step → {parameter}",
            has_doc,
            add
        ),
        cmd!(
            "param.set",
            "Set Parameter",
            [],
            None,
            "{name, value?, min?, max?, step?, source? (null or \"\": none), rename?: a new name (bindings that read it follow)} change a parameter, as one undo step; every object bound to it redraws → {parameter}",
            has_doc,
            set
        ),
        cmd!(
            "param.remove",
            "Remove Parameter",
            [],
            None,
            "{name} remove a parameter, as one undo step; fields bound to it use their own values (with a warning) until a parameter of that name exists again → {removed, stillBound: objects whose bindings read it}",
            has_doc,
            remove
        ),
        cmd!(
            query "param.list",
            "List Parameters",
            [],
            None,
            "{} → {parameters: [{name, type, value (its own), min?, max?, step?, source?, effective (what bindings read: the active data set's value when it has one), fromDataSet, warning?, objects (bound objects reading it)}], dataSet: the active data set | null, clock: 0}",
            has_doc,
            list
        ),
        cmd!(
            "modifier.bind",
            "Bind Modifier Field",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, field: a number field of the row (camelCase, as modifier.add takes it), binding: {param: name} | {expr: text (i, n, t = the clock (0 for now), value = the field's own value, parameter names; at most 256 characters)}} drive a field by a parameter or an expression, as one undo step; transformEach/randomize amounts give each copy its own value from an expression reading i or n; results are clamped to the field's range; on failure the field's own value is used, with a warning → {perCopy, bound: [{id, modifier}]}",
            has_selection,
            bind
        ),
        cmd!(
            "modifier.unbind",
            "Unbind Modifier Field",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, field?: (default: every bound field of the row)} drop a field's binding, as one undo step (the field keeps its own value) → {removed}",
            has_selection,
            unbind
        ),
    ]
}

fn name_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    text_param(p, "name", cmd)?.ok_or_else(|| bad(cmd, "`name` required (a parameter name)"))
}

/// `p`'s keys other than `skip`, as a parameter's props.
fn props_without(p: &Value, skip: &[&str]) -> Map<String, Value> {
    p.as_object().map(|o| o.iter().filter(|(k, _)| !skip.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "param.add";
    let name = name_param(CMD, p)?.trim().to_string();
    let mut params = Parameters::of_doc(&s.doc()?.doc);
    if params.list.len() >= vectorcraft_modifiers::params::MAX_PARAMS {
        return Err(bad(CMD, format!("a document holds at most {} parameters", vectorcraft_modifiers::params::MAX_PARAMS)));
    }
    params.check_new_name(&name, None).map_err(|e| bad(CMD, e))?;
    let param = Parameter::new(&name, &props_without(p, &["name"])).map_err(|e| bad(CMD, e))?;
    let out = param.to_value();
    params.list.push(param);
    s.edit("Add Parameter", |doc, _| {
        params.store(doc);
        Ok(())
    })?;
    Ok(json!({"parameter": out}))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "param.set";
    let name = name_param(CMD, p)?.to_string();
    let rename = text_param(p, "rename", CMD)?.map(|r| r.trim().to_string());
    let doc = &s.doc()?.doc;
    let mut params = Parameters::of_doc(doc);
    let param = params.get_mut(&name).ok_or_else(|| bad(CMD, format!("no parameter named `{name}`")))?;
    param.set(&props_without(p, &["name", "rename"])).map_err(|e| bad(CMD, e))?;
    let old = param.name.clone();
    let renamed = match rename {
        Some(new) if new != old => {
            params.check_new_name(&new, Some(&old)).map_err(|e| bad(CMD, e))?;
            if let Some(param) = params.get_mut(&old) {
                param.name.clone_from(&new);
            }
            Some((old.clone(), new))
        }
        _ => None,
    };
    // The objects whose bindings read the old name follow the rename.
    let rewrites = match &renamed {
        Some((from, to)) => renamed_stacks(doc, from, to).map_err(|e| bad(CMD, e))?,
        None => Vec::new(),
    };
    let out = params.get(renamed.as_ref().map_or(old.as_str(), |(_, to)| to.as_str())).map(Parameter::to_value).unwrap_or(Value::Null);
    s.edit("Set Parameter", |doc, _| {
        params.store(doc);
        for (id, src) in rewrites {
            if let Some(n) = doc.node_mut(id) {
                vectorcraft_modifiers::set_node_source(n, Some(src));
            }
        }
        Ok(())
    })?;
    Ok(json!({"parameter": out}))
}

/// Every object of `doc` whose bindings read parameter `from`, with its stack source rewritten to
/// read `to`. Refused when a rewritten expression would be too long.
fn renamed_stacks(doc: &Document, from: &str, to: &str) -> std::result::Result<Vec<(NodeId, String)>, String> {
    let mut out = Vec::new();
    let mut failed = None;
    doc.walk(|n| {
        let Some(Ok(stack)) = vectorcraft_modifiers::node_stack(n) else { return };
        if !stack.modifiers.iter().any(|m| m.bind.values().any(|b| b.reads(from))) {
            return;
        }
        let mut stack = (*stack).clone();
        for m in &mut stack.modifiers {
            for b in m.bind.values_mut() {
                if let Some(r) = b.renamed(from, to) {
                    if matches!(&r, Binding::Expr(e) if e.len() > vectorcraft_modifiers::expr::MAX_EXPR_LEN) {
                        failed = Some(format!(
                            "renaming would make an expression of object {} longer than {} characters",
                            n.id.0,
                            vectorcraft_modifiers::expr::MAX_EXPR_LEN
                        ));
                    }
                    *b = r;
                }
            }
        }
        out.push((n.id, vectorcraft_modifiers::to_json(&stack)));
    });
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// The objects of `doc` whose bindings read parameter `name` (any case).
pub(crate) fn objects_reading(doc: &Document, name: &str) -> Vec<NodeId> {
    let mut out = Vec::new();
    doc.walk(|n| {
        if let Some(Ok(stack)) = vectorcraft_modifiers::node_stack(n)
            && stack.modifiers.iter().any(|m| m.bind.values().any(|b| b.reads(name)))
        {
            out.push(n.id);
        }
    });
    out
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "param.remove";
    let name = name_param(CMD, p)?.to_string();
    let doc = &s.doc()?.doc;
    let mut params = Parameters::of_doc(doc);
    let at = params.position(&name).ok_or_else(|| bad(CMD, format!("no parameter named `{name}`")))?;
    let removed = params.list.remove(at);
    let still: Vec<u64> = objects_reading(doc, &removed.name).iter().map(|i| i.0).collect();
    s.edit("Remove Parameter", |doc, _| {
        params.store(doc);
        Ok(())
    })?;
    Ok(json!({"removed": removed.name, "stillBound": still}))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let params = Parameters::of_doc(doc);
    let scope = Scope::of_doc(doc);
    let rows: Vec<Value> = params
        .list
        .iter()
        .map(|param| {
            let mut v = param.to_value();
            if let (Some(o), Some(r)) = (v.as_object_mut(), scope.get(&param.name)) {
                o.insert("effective".into(), r.value.to_json());
                o.insert("fromDataSet".into(), json!(r.from_dataset));
                if let Some(w) = &r.warning {
                    o.insert("warning".into(), json!(w));
                }
                o.insert("objects".into(), json!(objects_reading(doc, &param.name).len()));
            }
            v
        })
        .collect();
    Ok(json!({"parameters": rows, "dataSet": doc.variables.active_dataset, "clock": 0}))
}

fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.bind";
    let field = text_param(p, "field", CMD)?.ok_or_else(|| bad(CMD, "`field` required (a number field of the row)"))?.to_string();
    let binding = Binding::from_request(p.get("binding").unwrap_or(&Value::Null)).map_err(|e| bad(CMD, e))?;
    let scope = Scope::of_doc(&s.doc()?.doc);
    let mut bound = Vec::new();
    let mut per_copy = false;
    edit(CMD, "Bind Modifier Field", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        let m = stack.modifiers.get_mut(at).ok_or_else(|| bad(CMD, "no such modifier"))?;
        per_copy |= m.bind_field(&field, binding.clone(), &scope).map_err(|e| bad(CMD, format!("object {}: {e}", id.0)))?;
        bound.push(json!({"id": id.0, "modifier": m.id}));
        Ok(())
    })?;
    Ok(json!({"perCopy": per_copy, "bound": bound}))
}

fn unbind(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.unbind";
    let field = text_param(p, "field", CMD)?.map(str::to_string);
    let mut removed = 0usize;
    // Count first: nothing bound is no undo step.
    {
        let doc = &s.doc()?.doc;
        let ids = super::objects(CMD, s, p)?;
        for id in ids {
            let stack = super::read(CMD, doc, id)?;
            let at = pick(CMD, p, &stack, id)?;
            removed += stack.modifiers.get(at).map_or(0, |m| match &field {
                Some(f) => usize::from(m.bind.contains_key(f)),
                None => m.bind.len(),
            });
        }
    }
    if removed == 0 {
        return Err(EngineError::Disabled(CMD.into(), "nothing bound there".into()));
    }
    edit(CMD, "Unbind Modifier Field", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        if let Some(m) = stack.modifiers.get_mut(at) {
            m.unbind_field(field.as_deref());
        }
        Ok(())
    })?;
    Ok(json!({"removed": removed}))
}
