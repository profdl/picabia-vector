//! Copy-to-shape targets: another layer's outline, read in the source layer's parent space.
//!
//! [`crate::evaluate`] alone has no document, so a `copyToShape` modifier evaluated through it
//! places no copies (with a warning). [`crate::evaluate_with`] takes a [`TargetResolver`];
//! [`DocTargets`] is the one built from a [`Document`] that the renderer, exports, hit testing and
//! commands use. A target is refused (no copies, a warning) when it is missing, a top-level layer,
//! the source itself, an ancestor or descendant of the source, or when its own stack points back at
//! the source, directly or through other targets ([`check_target`]). The engine refuses
//! such targets when they are set; evaluation ignores them in files that have them anyway.
//!
//! The target's outline is its own geometry in document space (a path's path; a group, compound
//! path or sublayer is the outlines of its visible members), not its modifier copies.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::BezPath;
use vectorcraft_geom::kurbo::PathEl;

use crate::cache;
use crate::node::node_source;

/// The most path elements a target outline may have (bigger outlines are refused).
pub const MAX_TARGET_ELEMENTS: usize = 100_000;
/// How deep a group target's layers are read.
const MAX_GROUP_DEPTH: usize = 16;
/// The most stacks a cycle check visits.
const MAX_CYCLE_VISITS: usize = 1024;
/// The most layers [`valid_targets`] lists.
pub const MAX_LISTED_TARGETS: usize = 10_000;

/// A resolved target: its outline in the source's parent space and a hash of it (part of the
/// evaluation cache key, so moving or editing the target re-evaluates its sources).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetShape {
    pub outline: BezPath,
    pub hash: u64,
}

impl TargetShape {
    /// A target shape for `outline`, hashed.
    pub fn new(outline: BezPath) -> TargetShape {
        let hash = hash_path(&outline);
        TargetShape { outline, hash }
    }
}

/// Finds a `copyToShape` modifier's target. `Err` is the warning to show (no copies are placed).
pub trait TargetResolver {
    fn resolve(&self, target: &str) -> Result<Arc<TargetShape>, String>;
}

/// No document: every target is unresolved.
pub struct NoTargets;

impl TargetResolver for NoTargets {
    fn resolve(&self, _target: &str) -> Result<Arc<TargetShape>, String> {
        Err("copy to shape needs the document to find its target; no copies".into())
    }
}

/// Targets read from `doc` for the layer `source`.
pub struct DocTargets<'a> {
    pub doc: &'a Document,
    pub source: NodeId,
}

impl<'a> DocTargets<'a> {
    pub fn new(doc: &'a Document, source: NodeId) -> Self {
        DocTargets { doc, source }
    }
}

impl TargetResolver for DocTargets<'_> {
    fn resolve(&self, target: &str) -> Result<Arc<TargetShape>, String> {
        let t = parse_target(target)?;
        check_target(self.doc, self.source, t)?;
        target_outline(self.doc, self.source, t).map(|o| Arc::new(TargetShape::new(o)))
    }
}

/// The object id a `target` text names (`"12"`, or `"#12"` as VectorCraft prints ids).
pub fn parse_target(target: &str) -> Result<NodeId, String> {
    let t = target.trim();
    t.strip_prefix('#')
        .unwrap_or(t)
        .parse::<u64>()
        .map(NodeId)
        .map_err(|_| format!("`{}` isn't an object id", crate::model::short(&serde_json::Value::String(target.into()))))
}

/// `n`'s display name for messages.
fn label(n: &Node) -> &str {
    n.name.as_deref().unwrap_or("the target")
}

/// Whether `a` is a strict ancestor of `b`.
fn is_ancestor(doc: &Document, a: NodeId, b: NodeId) -> bool {
    a != b && doc.ancestry(b).is_some_and(|chain| chain.contains(&a))
}

/// Whether layer `target` can be layer `source`'s copy-to-shape target; `Err` says why not.
pub fn check_target(doc: &Document, source: NodeId, target: NodeId) -> Result<(), String> {
    let Some(t) = doc.node(target) else { return Err("the target layer is gone (deleted, or not in this file); no copies".into()) };
    if doc.parent_of(target).is_none() {
        return Err("a top-level layer can't be a copy-to-shape target".into());
    }
    if target == source {
        return Err("an object can't copy onto itself".into());
    }
    if is_ancestor(doc, target, source) {
        return Err(format!("\"{}\" contains this object, so it can't be its target", label(t)));
    }
    if is_ancestor(doc, source, target) {
        return Err(format!("\"{}\" is inside this object, so it can't be its target", label(t)));
    }
    if reaches(doc, target, source) {
        return Err(format!("\"{}\" copies onto this object itself (directly or through other objects), so it can't be its target", label(t)));
    }
    Ok(())
}

/// Whether following copy-to-shape targets from `from`'s stack reaches `to`.
fn reaches(doc: &Document, from: NodeId, to: NodeId) -> bool {
    let mut seen = BTreeSet::new();
    let mut todo = vec![from];
    while let Some(id) = todo.pop() {
        if !seen.insert(id) || seen.len() > MAX_CYCLE_VISITS {
            continue;
        }
        for t in stack_targets(doc.node(id)) {
            if t == to {
                return true;
            }
            todo.push(t);
        }
    }
    false
}

/// The layer ids `n`'s stack copies onto (enabled modifiers; unreadable ids left out).
pub fn stack_targets(n: Option<&Node>) -> Vec<NodeId> {
    let Some(src) = n.and_then(node_source) else { return Vec::new() };
    let Ok(stack) = cache::parsed(src) else { return Vec::new() };
    stack.targets().filter_map(|(_, t)| parse_target(t).ok()).collect()
}

/// The objects `source` can copy onto: the objects in the document passing [`check_target`] that
/// have an outline, in layer order (at most [`MAX_LISTED_TARGETS`]).
pub fn valid_targets(doc: &Document, source: NodeId) -> Vec<NodeId> {
    let mut ids = Vec::new();
    doc.walk(|n| ids.push(n.id));
    ids.into_iter()
        .filter(|id| check_target(doc, source, *id).is_ok())
        .filter(|id| doc.node(*id).and_then(|n| local_outline(n, 0)).is_some_and(|p| !p.elements().is_empty()))
        .take(MAX_LISTED_TARGETS)
        .collect()
}

/// `target`'s outline in document space (where `source`'s geometry also lives).
fn target_outline(doc: &Document, _source: NodeId, target: NodeId) -> Result<BezPath, String> {
    let t = doc.node(target).ok_or("the target object is gone; no copies")?;
    let out = local_outline(t, 0).ok_or_else(|| format!("\"{}\" has no outline to copy onto", label(t)))?;
    if out.elements().len() > MAX_TARGET_ELEMENTS {
        return Err(format!("\"{}\" has more than {MAX_TARGET_ELEMENTS} path points", label(t)));
    }
    let finite = out.elements().iter().all(|e| el_points(e).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    if !finite || out.elements().is_empty() {
        return Err(format!("\"{}\" has no outline to copy onto", label(t)));
    }
    Ok(out)
}

/// `n`'s outline in document space: its path, or for a group, compound path or sublayer its
/// visible members' outlines.
fn local_outline(n: &Node, depth: usize) -> Option<BezPath> {
    if let NodeKind::Path { path, .. } = &n.kind {
        return Some(path.to_bezpath());
    }
    if depth >= MAX_GROUP_DEPTH {
        return None;
    }
    let mut out = BezPath::new();
    for c in n.children().map(Vec::as_slice).unwrap_or_default() {
        if !c.visible {
            continue;
        }
        let Some(p) = local_outline(c, depth + 1) else { continue };
        if out.elements().len() + p.elements().len() > MAX_TARGET_ELEMENTS {
            break;
        }
        out.extend(p.elements().iter().copied());
    }
    (!out.elements().is_empty()).then_some(out)
}

fn el_points(e: &PathEl) -> Vec<vectorcraft_geom::Point> {
    match *e {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => Vec::new(),
    }
}

fn hash_path(p: &BezPath) -> u64 {
    let mut h = DefaultHasher::new();
    for e in p.elements() {
        std::mem::discriminant(e).hash(&mut h);
        for q in el_points(e) {
            q.x.to_bits().hash(&mut h);
            q.y.to_bits().hash(&mut h);
        }
    }
    h.finish()
}
