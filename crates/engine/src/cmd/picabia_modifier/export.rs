//! PICABIA: modifier copies in exports.
//!
//! The renderer draws the copies, and every vector exporter that bakes the document
//! (`vectorcraft_effects::bake_document`: SVG, PDF, print, EPS, DXF, EMF, WMF) writes them through
//! the hooks registered in `Session::new` (`vectorcraft_effects::picabia`). The written copies
//! carry no stack, so reading the file back doesn't copy them again.
//!
//! Where the stack survives: the native file and its template keep it; SVG and PDF with Preserve
//! Editing, a modern `.ai` (a PDF with editing data) and an EPS VectorCraft wrote carry the native
//! document, which reopens live. Every other format drops it, and says so ([`export_warning`]).
//!
//! The writers that read the tree themselves get a copy of the document with the copies baked as
//! plain objects ([`vectorcraft_render::picabia::baked`]): plug-in exporters ([`baked_export`])
//! and legacy .ai ([`bake_copies`]).

use serde_json::Value;
use vectorcraft_doc::Document;

use crate::Result;
use crate::cmd::fileio::{Encoded, Format};

/// The warning for a file that drops `doc`'s stacks (the copies are written as plain objects).
fn dropped(doc: &Document) -> Option<String> {
    vectorcraft_render::picabia::has_stacks(doc)
        .then(|| "modifier copies are written as plain objects: the file won't reopen with them live".to_string())
}

/// The warning for SVG and PDF written without Preserve Editing.
fn dropped_unless_preserved(doc: &Document) -> Option<String> {
    vectorcraft_render::picabia::has_stacks(doc)
        .then(|| "modifier copies are written as plain objects: turn on Preserve Editing to reopen them live".to_string())
}

/// `doc` with its modifier copies baked in place, and the warning to give (nothing to do without
/// any).
pub(crate) fn bake_copies(doc: &mut Document) -> Option<String> {
    let w = dropped(doc);
    if let Some(d) = vectorcraft_render::picabia::baked(doc) {
        *doc = d;
    }
    w
}

/// `write` of `doc` with its modifier copies baked as plain objects.
pub(crate) fn baked_export(doc: &Document, write: impl FnOnce(&Document) -> Result<Encoded>) -> Result<Encoded> {
    let Some(d) = vectorcraft_render::picabia::baked(doc) else { return write(doc) };
    let mut enc = write(&d)?;
    enc.warnings.extend(dropped(doc));
    Ok(enc)
}

/// The warning for format `f` written with params `p` when `doc` has modifier stacks and the file
/// drops them. Raster formats draw the copies and keep nothing to warn about; legacy .ai warns
/// itself ([`bake_copies`]).
pub(crate) fn export_warning(doc: &Document, f: &Format, p: &Value, warnings: &mut Vec<String>) {
    let preserve = |default: bool| p.get("preserveEditing").and_then(Value::as_bool).unwrap_or(default);
    let w = match f.id {
        _ if f.raster => None,
        "vectorcraft" | "template" | "eps" | "ai" | "ait" | "txt" => None,
        "svg" | "svgz" if !preserve(false) => dropped_unless_preserved(doc),
        "pdf" if !preserve(true) => dropped_unless_preserved(doc),
        "svg" | "svgz" | "pdf" => None,
        _ => dropped(doc),
    };
    warnings.extend(w);
}
