//! PICABIA: an object with a modifier stack draws as its copies.
//!
//! The object becomes "art" the way VectorCraft's art-replacing effects do (Crop Marks, Vector
//! Halftone): [`art`] is a group that takes the object's place and identity, holding one copy of
//! the object per modifier instance, its geometry transformed by the instance transform
//! (`Node::transform`, strokes, effects, corners and patterns scaling with it, as
//! `modifier.apply` bakes it). The renderer draws it through `fx::has_object_fx` and the `fx_art`
//! chain; bounds, culling and dirty regions measure it in `fx::cull_bounds_in`; every vector
//! exporter writes it through `vectorcraft_effects::picabia` (registered from the engine); hit
//! testing tries each copy through `vectorcraft_doc::hit::set_copies`.
//!
//! # Evaluation order
//! Copies are made from the object's geometry first; each copy then draws with the object's
//! fills, strokes and effects (geometry effects and raster effects such as a drop shadow apply to
//! each copy). The object's transparency (opacity, blending mode, isolation, opacity mask) goes
//! on the group: the copies are composited together, once. No copy carries the stack, so the art
//! never recurses; members of a copied group keep their own stacks.
//!
//! An object whose stack makes no copies (no active row, one unmoved copy, a copy-to-shape target
//! that is missing) has no art: it draws as itself.

use std::sync::{Arc, Mutex, PoisonError};

use vectorcraft_doc::{Document, Node, Scaling};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_modifiers::Output;

/// How many arts are kept, the most recently used first.
const KEPT: usize = 16;

/// The most copies an object's art may draw with the copies its members' own stacks make inside
/// each of them; past this the members draw once in each copy.
pub const MAX_NESTED_COPIES: usize = vectorcraft_modifiers::MAX_INSTANCES_PER_NODE;

/// An art kept: the object it was made for, the evaluation it was made from (the modifiers
/// crate's cache gives a new one when the stack, the bounds or a copy-to-shape target changes),
/// and the art.
type Kept = (Node, Arc<Output>, Arc<Node>);

static CACHE: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// Whether `n` has a stack with an active row (an attribute lookup and a cached parse: cheap
/// enough for every object on every frame). It may still make no copies ([`art`]).
pub fn has_stack(n: &Node) -> bool {
    matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.is_active())
}

/// Whether `n`'s stack copies onto another object: its art then changes with that object, so
/// caches kept by `n`'s identity alone don't hold it.
pub fn has_targets(n: &Node) -> bool {
    matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.targets().next().is_some())
}

/// The transform each copy `node` draws, or `None` when it draws once as itself.
fn copies(doc: &Document, n: &Node) -> Option<Arc<Output>> {
    if !has_stack(n) {
        return None;
    }
    let out = vectorcraft_modifiers::evaluate_node_in_doc(doc, n)?;
    let unmoved = matches!(out.instances.as_slice(), [one] if one.transform == Affine::IDENTITY);
    (!out.instances.is_empty() && !unmoved).then_some(out)
}

/// What a copy's transform scales besides its geometry: everything, as a copy drawn bigger.
fn scaling() -> Scaling {
    Scaling { strokes: true, effects: Some(vectorcraft_effects::scale_effect), keep_type_strokes: false, keep_corners: false, patterns: true }
}

/// `n`'s art in `doc`: the group of its copies (see the module docs), or `None` when it draws as
/// itself. Kept while `n` and its evaluation stay the same.
pub fn art(doc: &Document, n: &Node) -> Option<Arc<Node>> {
    let out = copies(doc, n)?;
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = cache.iter().position(|(node, o, _)| Arc::ptr_eq(o, &out) && node == n) {
            let kept = cache.remove(at);
            let art = kept.2.clone();
            cache.insert(0, kept);
            return Some(art);
        }
    }
    let art = Arc::new(make(n, &out));
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.insert(0, (n.clone(), out, art.clone()));
    cache.truncate(KEPT);
    Some(art)
}

/// [`art`] as an owned node, for the renderer's art chain and the exporters' bake: an object with
/// an active stack that makes no copies is itself without the stack (so drawing it doesn't come
/// back here); `None` without an active stack.
pub fn art_node(doc: &Document, n: &Node) -> Option<Node> {
    if !has_stack(n) {
        return None;
    }
    Some(art(doc, n).map(Arc::unwrap_or_clone).unwrap_or_else(|| {
        let mut plain = n.clone();
        vectorcraft_modifiers::set_node_source(&mut plain, None);
        plain
    }))
}

/// The group of `n`'s copies for `out`.
fn make(n: &Node, out: &Output) -> Node {
    let mut object = n.clone();
    let mut g = Node::group(n.id, vec![]);
    (g.visible, g.locked) = (n.visible, n.locked);
    g.name = object.name.take();
    g.opacity = std::mem::replace(&mut object.opacity, 1.0);
    g.blend = std::mem::take(&mut object.blend);
    g.isolate = std::mem::take(&mut object.isolate);
    g.extras_mut().mask = object.extras_mut().mask.take();
    g.extras_mut().attrs = object.extras_mut().attrs.take();
    // The art is drawn and written as it is: it carries no stack (a written copy that did would
    // be copied again when the file is read back).
    vectorcraft_modifiers::set_node_source(&mut g, None);
    // Members with stacks of their own copy inside every copy: within one budget.
    let nested = nested_copies(&object);
    if nested > 0 && out.instances.len().saturating_mul(nested) > MAX_NESTED_COPIES {
        strip_nested(&mut object, 0);
    }
    let sc = scaling();
    let copies = out
        .instances
        .iter()
        .filter(|i| is_finite(i.transform))
        .map(|i| {
            let mut c = object.clone();
            if i.transform != Affine::IDENTITY {
                c.transform(i.transform, sc);
            }
            Arc::new(c)
        })
        .collect();
    if let Some(ch) = g.children_mut() {
        *ch = copies;
    }
    g
}

/// How many copies the members of `n` (not `n` itself) draw through their own stacks.
fn nested_copies(n: &Node) -> usize {
    let mut total = 0usize;
    for c in n.children().into_iter().flatten() {
        c.walk(&mut |m| {
            if has_stack(m) {
                let k = vectorcraft_modifiers::node_instances(m).map_or(1, |o| o.instances.len());
                total = total.saturating_add(k);
            }
        });
    }
    total
}

/// `n`'s members without their stacks.
fn strip_nested(n: &mut Node, depth: usize) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        let c = Arc::make_mut(c);
        vectorcraft_modifiers::set_node_source(c, None);
        strip_nested(c, depth + 1);
    }
}

fn is_finite(a: Affine) -> bool {
    a.as_coeffs().iter().all(|c| c.is_finite())
}

/// For `vectorcraft_doc::hit::set_copies`: the art of `n` (its copies, each hit as `n`).
pub fn hit_copies(symbols: &vectorcraft_doc::SymbolBoxes, n: &Node) -> Option<Arc<Node>> {
    art(symbols.document(), n)
}

/// What may draw differently in `new` than in `old` because a copy-to-shape target changed while
/// its source didn't: the sources' painted bounds before and after.
pub(crate) fn dependents(old: &Document, new: &Document) -> Option<Rect> {
    let (os, ns) = (old.symbol_boxes(), new.symbol_boxes());
    let mut area: Option<Rect> = None;
    new.walk(|n| {
        if !has_targets(n) {
            return;
        }
        let moved = vectorcraft_modifiers::stack_targets(Some(n)).iter().any(|t| match (old.node(*t), new.node(*t)) {
            (Some(a), Some(b)) => !std::ptr::eq(a, b),
            (None, None) => false,
            _ => true,
        });
        if !moved {
            return;
        }
        area = vectorcraft_geom::union_opt(area, crate::brush_fx::cull_bounds(&ns, n));
        area = vectorcraft_geom::union_opt(area, old.node(n.id).and_then(|o| crate::brush_fx::cull_bounds(&os, o)));
    });
    area
}

#[cfg(test)]
#[path = "picabia_tests.rs"]
mod tests;
