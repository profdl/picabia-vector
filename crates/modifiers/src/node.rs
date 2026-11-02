//! Entry points for a document node: read its stack from its foreign attribute and evaluate it,
//! cached.
//!
//! VectorCraft objects keep their geometry in document space (no per-object transform), so a
//! node's "parent space" in the instance convention is document space and its bounds are its
//! geometric bounds there.

use std::sync::Arc;

use vectorcraft_doc::{Document, ForeignAttribute, Node};
use vectorcraft_geom::Rect;

use crate::eval::Output;
use crate::model::Stack;
use crate::params::Scope;
use crate::target::{DocTargets, TargetResolver};
use crate::{ModError, PLUGIN_KEY, STACK_NS, STACK_PREFIX, cache};

/// `node`'s stack source: the value of its `picabia.modifiers` foreign attribute, if any.
pub fn node_source(node: &Node) -> Option<&str> {
    let attrs = node.extras().attrs.as_deref()?;
    attrs.foreign.iter().find(|a| a.ns == STACK_NS && a.name == PLUGIN_KEY).map(|a| a.value.as_str())
}

/// Set (`Some`) or remove (`None`) `node`'s stack source. Other foreign attributes are kept.
pub fn set_node_source(node: &mut Node, src: Option<String>) {
    if src.is_none() && node_source(node).is_none() {
        return;
    }
    node.edit_attrs(|a| {
        let at = a.foreign.iter().position(|f| f.ns == STACK_NS && f.name == PLUGIN_KEY);
        match (src, at) {
            (Some(value), Some(i)) => {
                if let Some(f) = a.foreign.get_mut(i) {
                    f.value = value;
                }
            }
            (Some(value), None) => {
                a.foreign.push(ForeignAttribute { ns: STACK_NS.into(), prefix: STACK_PREFIX.into(), name: PLUGIN_KEY.into(), value });
            }
            (None, Some(i)) => {
                a.foreign.remove(i);
            }
            (None, None) => {}
        }
    });
}

/// Whether `node` has modifier data (an attribute lookup; doesn't parse).
pub fn has_modifiers(node: &Node) -> bool {
    node_source(node).is_some()
}

/// `node`'s stack: `None` without modifier data, `Some(Err)` when the data is damaged (the layer
/// draws plainly; show the error in the panel). Cached by source text.
pub fn node_stack(node: &Node) -> Option<Result<Arc<Stack>, ModError>> {
    node_source(node).map(cache::parsed)
}

/// `node`'s geometric bounding box in document space, whose centre is the modifiers' pivot. A node
/// with no geometry (an empty group) or non-finite bounds reads as the empty rectangle at the
/// origin. Without the document a symbol instance is measured as VectorCraft's 20 × 20 stand-in
/// square: [`node_bounds_in_doc`] measures its art.
pub fn node_bounds(node: &Node) -> Rect {
    finite_or_zero(node.geometric_bounds())
}

/// [`node_bounds`] with symbol instances measured by their symbol's art in `doc`, as the selection
/// box is.
pub fn node_bounds_in_doc(doc: &Document, node: &Node) -> Rect {
    finite_or_zero(node.geometric_bounds_with(&doc.symbol_boxes()))
}

fn finite_or_zero(r: Option<Rect>) -> Rect {
    r.filter(|r| r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()).unwrap_or(Rect::ZERO)
}

/// [`evaluate_node_in`] with [`node_bounds`].
pub fn evaluate_node(node: &Node) -> Option<Arc<Output>> {
    evaluate_node_in(node, node_bounds(node))
}

/// Evaluate `node`'s stack for `bounds` (its bounding box in document space), cached by (source,
/// bounds). `None` means "draw the layer normally": no modifier data, damaged data, or no active
/// modifier. A `Some` always has at least one instance.
pub fn evaluate_node_in(node: &Node, bounds: Rect) -> Option<Arc<Output>> {
    let src = node_source(node)?;
    cache::evaluated(src, bounds).ok().filter(|o| !o.instances.is_empty())
}

/// [`evaluate_node_in`] with `targets` resolving `copyToShape` targets; the cache key includes
/// each target's outline hash.
pub fn evaluate_node_with(node: &Node, bounds: Rect, targets: &dyn TargetResolver) -> Option<Arc<Output>> {
    let src = node_source(node)?;
    cache::evaluated_with(src, bounds, targets).ok().filter(|o| !o.instances.is_empty())
}

/// [`evaluate_node`] for an object of `doc`, resolving its `copyToShape` targets there
/// ([`DocTargets`]): what the renderer, exports, hit testing, commands and the panel use.
pub fn evaluate_node_in_doc(doc: &Document, node: &Node) -> Option<Arc<Output>> {
    let src = node_source(node)?;
    let scope = doc_scope_for(doc, src);
    cache::evaluated_in(src, node_bounds_in_doc(doc, node), &DocTargets::new(doc, node.id), &scope).ok().filter(|o| !o.instances.is_empty())
}

/// `doc`'s parameters when stack source `src` has bindings (they're read only then).
fn doc_scope_for(doc: &Document, src: &str) -> std::sync::Arc<Scope> {
    match cache::parsed(src) {
        Ok(s) if s.has_bindings() => Scope::of_doc(doc),
        _ => std::sync::Arc::default(),
    }
}

/// The parameters `node`'s stack reads, resolved in `doc`: a hash that changes exactly when a value
/// its bindings read does (0 without bindings). What repaints a bound object whose parameter
/// changed although the object didn't.
pub fn bound_key(doc: &Document, node: &Node) -> u64 {
    let Some(src) = node_source(node) else { return 0 };
    match cache::parsed(src) {
        Ok(s) if s.has_bindings() => Scope::of_doc(doc).key(s.bound_names().iter().map(String::as_str)),
        _ => 0,
    }
}

/// Whether `node`'s stack has bindings (an attribute lookup and a cached parse): its drawing then
/// depends on the document's parameters, not on the object alone.
pub fn has_bindings(node: &Node) -> bool {
    node_stack(node).is_some_and(|s| s.is_ok_and(|s| s.has_bindings()))
}

/// [`node_instances`] for an object of `doc` (copy-to-shape targets resolved there).
pub fn node_instances_in_doc(doc: &Document, node: &Node) -> Option<Arc<Output>> {
    if !has_modifiers(node) {
        return None;
    }
    evaluate_node_in_doc(doc, node)
}

/// `node`'s instance transforms (document space, applied to its geometry), or `None` to draw it
/// once as usual. Costs an attribute lookup for an object without modifiers and a cache hit for
/// one with. Without a document `copyToShape` places no copies: the renderer calls
/// [`node_instances_in_doc`].
pub fn node_instances(node: &Node) -> Option<Arc<Output>> {
    if !has_modifiers(node) {
        return None;
    }
    evaluate_node(node)
}
