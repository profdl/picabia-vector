//! PICABIA: modifier copies in exports.
//!
//! The renderer draws the copies, and every vector exporter that bakes the document
//! (`vectorcraft_effects::bake_document`: SVG, PDF, print, EPS, DXF, EMF, WMF) writes them through
//! the hooks registered in `Session::new` (`vectorcraft_effects::picabia`). The written copies
//! carry no stack, so reading the file back doesn't copy them again; SVG and PDF with Preserve
//! Editing keep the live stack in their editing data, as the native file does.
//!
//! The writers that read the tree themselves get a copy of the document with the copies baked as
//! plain objects ([`vectorcraft_render::picabia::baked`]): plug-in exporters ([`baked_export`])
//! and legacy .ai ([`bake_copies`]). Every format but the native one says so in a warning
//! ([`export_warning`]).

use vectorcraft_doc::Document;

use crate::Result;
use crate::cmd::fileio::{Encoded, Format};

/// `doc` with its modifier copies baked in place (nothing to do without any).
pub(crate) fn bake_copies(doc: &mut Document) {
    if let Some(d) = vectorcraft_render::picabia::baked(doc) {
        *doc = d;
    }
}

/// `write` of `doc` with its modifier copies baked as plain objects.
pub(crate) fn baked_export(doc: &Document, write: impl FnOnce(&Document) -> Result<Encoded>) -> Result<Encoded> {
    let Some(d) = vectorcraft_render::picabia::baked(doc) else { return write(doc) };
    let mut enc = write(&d)?;
    enc.warnings.extend(vectorcraft_render::picabia::bake_warning(doc));
    Ok(enc)
}

/// The warning for format `f` when `doc` has modifier stacks: every format but the native one
/// (and its template) writes the copies as plain objects. Raster formats draw the copies and
/// keep nothing to warn about.
pub(crate) fn export_warning(doc: &Document, f: &Format, warnings: &mut Vec<String>) {
    if f.raster || matches!(f.id, "vectorcraft" | "template") {
        return;
    }
    warnings.extend(vectorcraft_render::picabia::bake_warning(doc));
}
