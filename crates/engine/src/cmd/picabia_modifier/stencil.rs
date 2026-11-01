//! PICABIA: `modifier.stencilInfo` and `modifier.stencilFromSelection`.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_modifiers::Modifier;

use super::{props_of, read, write};
use crate::{EngineError, Result, Session};

/// `modifier.stencilInfo`: the stencil's swatches, stats, islands, problems and the rest, computed
/// now when the kept one is out of date.
pub(super) fn stencil_info(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.stencilInfo";
    let id = super::one_object(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let n = doc.node(id).ok_or_else(|| super::super::bad(CMD, format!("no object with id {}", id.0)))?;
    if !crate::picabia_stencil::is_stencil(n) {
        return Ok(json!({"id": id.0, "stencil": null}));
    }
    let Some(sheet) = crate::picabia_stencil::ensure_one(doc, id) else {
        // A stencil row on an object without a size to cut.
        return Ok(json!({"id": id.0, "stencil": null, "warnings": ["the object has no size to cut"]}));
    };
    let mut out = crate::picabia_stencil::info_json(&sheet.info);
    out["id"] = json!(id.0);
    Ok(out)
}

/// The objects `p` names (`ids`/`id`, else the selection's top-level objects) in paint order.
fn chosen_roots(cmd: &str, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let given = p.get("ids").is_some_and(|v| !v.is_null()) || p.get("id").is_some_and(|v| !v.is_null());
    let ids = if given {
        let ids = super::super::targets(s, p).map_err(|e| super::super::bad(cmd, e.to_string()))?;
        for id in &ids {
            st.doc.node(*id).ok_or_else(|| super::super::bad(cmd, format!("no object with id {}", id.0)))?;
        }
        // Paint order: by place in the tree.
        let paths = st.doc.index_paths(ids.iter().copied());
        let mut ids = ids;
        ids.sort_by(|a, b| paths.get(a).cmp(&paths.get(b)));
        ids.dedup();
        ids
    } else {
        st.selection.in_paint_order(&st.doc)
    };
    let roots = super::super::edit::roots_of(&st.doc, ids);
    for id in &roots {
        if st.doc.node(*id).is_some_and(|n| n.is_layer()) {
            return Err(super::super::bad(cmd, format!("{} is a layer; select the objects to make a stencil of", id.0)));
        }
    }
    Ok(roots)
}

/// `modifier.stencilFromSelection`: one object takes a stencil row itself; several are grouped
/// first (named "Stencil"), in one undo step with the row.
pub(super) fn stencil_from_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.stencilFromSelection";
    let mut template = Modifier::new("", "stencil").ok_or_else(|| super::super::bad(CMD, "no stencil kind"))?;
    if let Some(props) = props_of(CMD, p, false)? {
        template.set_props(props).map_err(|e| super::super::bad(CMD, e.to_string()))?;
    }
    let ids = chosen_roots(CMD, s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "select the objects to make a stencil of".into()));
    }
    if let [one] = ids.as_slice() {
        let stack = read(CMD, &s.doc()?.doc, *one)?;
        if stack.stencil().is_some() {
            return Err(EngineError::Disabled(CMD.into(), format!("object {} is a stencil already", one.0)));
        }
    }
    let (id, modifier) = s.edit("Make Stencil", |d, sel| {
        let id = match ids.as_slice() {
            [one] => *one,
            many => {
                let gid = super::super::object::group_nodes(d, many)?;
                if let Some(g) = d.node_mut(gid) {
                    g.name = Some("Stencil".into());
                }
                gid
            }
        };
        let mut stack = read(CMD, d, id)?;
        let m = stack.insert(usize::MAX, template.clone()).map_err(|e| super::super::bad(CMD, format!("object {}: {e}", id.0)))?;
        let modifier = m.id.clone();
        write(CMD, d, id, &stack)?;
        sel.set([id]);
        Ok((id, modifier))
    })?;
    Ok(json!({"id": id.0, "modifier": modifier}))
}
