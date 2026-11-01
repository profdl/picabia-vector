//! PICABIA: an object's modifier copies are clicked, hovered and marqueed as the object.
//!
//! Modifier stacks are read above this crate, so the engine registers how they are found
//! ([`set_copies`]). A click on an object with copies tries the object itself under each copy's
//! transform, the front copy first (the point mapped back, the tolerance scaled): no copy is
//! built for a click. A marquee touches the object through the group of its copies, and Direct
//! Selection's marquee takes the members of those copies (each keeps its member's id).
//!
//! Without stacks in the document, a click costs one attribute lookup per object tried, and one
//! kept-flag lookup per container whose own reach misses the click ([`CopyHooks::holds`]).

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

use vectorcraft_geom::{Affine, Point};

use super::{Hit, HitOptions};
use crate::{Node, NodeId, SymbolBoxes};

/// How modifier copies are found.
#[derive(Clone, Copy)]
pub struct CopyHooks {
    /// Whether an object has an active stack (an attribute lookup).
    pub has: fn(&Node) -> bool,
    /// Whether something below a container has an active stack (kept per container).
    pub holds: fn(&SymbolBoxes, &Node) -> bool,
    /// The transforms of an object's copies, the front one last; `None` when it draws once.
    pub transforms: fn(&SymbolBoxes, &Node) -> Option<Arc<[Affine]>>,
    /// The group of an object's copies, for marquees.
    pub art: fn(&SymbolBoxes, &Node) -> Option<Arc<Node>>,
}

static HOOKS: OnceLock<CopyHooks> = OnceLock::new();

thread_local! {
    /// The objects being tried under one of their copies' transforms: drawn once there.
    static PLAIN: RefCell<Vec<NodeId>> = const { RefCell::new(Vec::new()) };
}

/// Register how modifier copies are found (the first registration stays).
pub fn set_copies(hooks: CopyHooks) {
    // Already set: the same engine, registered by another session.
    let _ = HOOKS.set(hooks);
}

fn plain(id: NodeId) -> bool {
    PLAIN.with(|p| p.borrow().contains(&id))
}

/// Whether something below `n` has copies, which may lie outside `n`'s own reach.
pub(super) fn holds_copies(symbols: &SymbolBoxes, n: &Node) -> bool {
    HOOKS.get().is_some_and(|h| (h.holds)(symbols, n))
}

/// The group of `n`'s copies, for marquees.
pub(super) fn copies(symbols: &SymbolBoxes, n: &Node) -> Option<Arc<Node>> {
    let h = HOOKS.get()?;
    if !(h.has)(n) || plain(n.id) {
        return None;
    }
    (h.art)(symbols, n)
}

/// Direct Selection's marquee in `n`, an object with copies: `collect` on each copy (each keeps
/// its members' ids), each id once. `false`: `n` has no copies, collect it as usual.
pub(super) fn leaves(symbols: &SymbolBoxes, n: &Node, out: &mut Vec<NodeId>, collect: &mut dyn FnMut(&Node, &mut Vec<NodeId>)) -> bool {
    let Some(art) = copies(symbols, n) else { return false };
    let mut found = Vec::new();
    for c in art.children().into_iter().flatten() {
        collect(c, &mut found);
    }
    for id in found {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    true
}

/// A click at `p` on `c`, an object with copies: `c` tried under each copy's transform, the front
/// one first, by `try_at` (the upstream click test of `c` as the only member of a group, at the
/// point mapped back, with the tolerance scaled). `None`: `c` has no copies, test it as usual;
/// `Some(None)`: no copy is hit (the caller goes on to the next object).
pub(super) fn hit(
    symbols: &SymbolBoxes,
    c: &Arc<Node>,
    p: Point,
    opt: HitOptions,
    try_at: &mut dyn FnMut(&Node, Point, HitOptions) -> Option<Hit>,
) -> Option<Option<Hit>> {
    let h = HOOKS.get()?;
    if !(h.has)(c) || plain(c.id) {
        return None;
    }
    let ts = (h.transforms)(symbols, c)?;
    let holder = Node::group(c.id, vec![c.clone()]);
    PLAIN.with(|pl| pl.borrow_mut().push(c.id));
    let mut found = None;
    for t in ts.iter().rev() {
        let det = t.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            continue;
        }
        let scale = det.abs().sqrt();
        let at = HitOptions { tol: opt.tol / scale, ..opt };
        if let Some(hit) = try_at(&holder, t.inverse() * p, at) {
            found = Some(hit);
            break;
        }
    }
    PLAIN.with(|pl| {
        pl.borrow_mut().pop();
    });
    Some(found)
}
