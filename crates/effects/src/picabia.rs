//! PICABIA: exports write every modifier copy.
//!
//! An object with a Picabia modifier stack draws as the group of its copies
//! (`vectorcraft_render::picabia::art`). The stack is read above this crate, so the engine
//! registers the predicate and the art builder here ([`set_hooks`]); [`crate::bake_document`]
//! then writes the copies, each baked as any object is, in every vector export (SVG, PDF, print,
//! EPS, DXF, EMF, WMF). The written copies carry no stack: a file read back draws them as they
//! are (formats with editing data, such as SVG and PDF with Preserve Editing, keep the live stack
//! there).

use std::sync::OnceLock;

use vectorcraft_doc::{Document, Node};

/// Whether an object has an active modifier stack (cheap: called for every object).
pub type HasHook = fn(&Node) -> bool;
/// The group of an object's modifier copies, `None` when it draws once as itself.
pub type ArtHook = fn(&Document, &Node) -> Option<Node>;

static HAS: OnceLock<HasHook> = OnceLock::new();
static ART: OnceLock<ArtHook> = OnceLock::new();

/// Register how stacks are found and their copies made (the first registration stays).
pub fn set_hooks(has: HasHook, art: ArtHook) {
    // Already set: the same engine, registered by another session.
    let _ = HAS.set(has);
    let _ = ART.set(art);
}

/// Whether `n` has an active modifier stack (false while nothing is registered).
pub fn has_copies(n: &Node) -> bool {
    HAS.get().is_some_and(|f| f(n))
}

/// The group of `n`'s copies in `d` ([`set_hooks`]), `None` when it draws as itself.
pub fn copies_art(d: &Document, n: &Node) -> Option<Node> {
    if !has_copies(n) {
        return None;
    }
    ART.get()?(d, n)
}
