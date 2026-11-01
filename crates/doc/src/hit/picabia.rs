//! PICABIA: an object's modifier copies are clicked, hovered and marqueed as the object.
//!
//! Modifier stacks are read above this crate, so the engine registers how an object's copies are
//! found ([`set_copies`]): the group of copies the renderer draws, each copy keeping the object's
//! id (and its members' ids), so a hit on a copy is a hit on the object.

use std::sync::{Arc, OnceLock};

use crate::{Node, SymbolBoxes};

/// The group of `n`'s modifier copies, `None` when it draws once as itself.
pub type Copies = fn(&SymbolBoxes, &Node) -> Option<Arc<Node>>;

/// Whether `n` has an active modifier stack (cheap: an attribute lookup).
pub type HasCopies = fn(&Node) -> bool;

static COPIES: OnceLock<(Copies, HasCopies)> = OnceLock::new();

/// Register how an object's modifier copies are found, and how an object with a stack is told
/// cheaply (the first registration stays).
pub fn set_copies(f: Copies, has: HasCopies) {
    // Already set: the same engine, registered by another session.
    let _ = COPIES.set((f, has));
}

/// Whether something in `n`'s subtree below `n` has copies, which may lie outside `n`'s own reach:
/// a click outside that reach must still look inside it.
pub(super) fn holds_copies(n: &Node) -> bool {
    let Some((_, has)) = COPIES.get() else { return false };
    let mut any = false;
    for c in n.children().into_iter().flatten() {
        c.walk(&mut |m| any |= has(m));
    }
    any
}

/// The group of `n`'s copies ([`set_copies`]), none while nothing is registered.
pub(super) fn copies(symbols: &SymbolBoxes, n: &Node) -> Option<Arc<Node>> {
    let (copies, has) = COPIES.get()?;
    if !has(n) {
        return None;
    }
    copies(symbols, n)
}

/// What a click tries for the child `c`: its copies, the front one first, or `c` itself.
pub(super) fn each_copy(symbols: &SymbolBoxes, c: Option<&Arc<Node>>) -> Vec<Arc<Node>> {
    let Some(c) = c else { return Vec::new() };
    match copies(symbols, c) {
        Some(art) => art.children().map(|ch| ch.iter().rev().cloned().collect()).unwrap_or_default(),
        None => vec![c.clone()],
    }
}
