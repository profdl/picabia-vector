//! PICABIA: outlines on an object's modifier copies: the hover highlight on every copy, and a
//! faint outline on a selected object's copies (the selection box stays on the object itself); a
//! selected stencil's tie bars and repeat cell.

use egui::Stroke;
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, BezPath};

use super::Xf;
use super::outline::stroke_path;

/// The most copies outlined for one object in a frame (as picabia-craft capped them).
pub(super) const MAX_OUTLINES: usize = 2_000;
/// The most path elements outlined for one object's copies in a frame.
const MAX_OUTLINE_ELEMENTS: usize = 200_000;

/// `outline` (the object's own) moved to each of `n`'s copies but the unmoved one: at most
/// [`MAX_OUTLINES`], and none when that would be more than [`MAX_OUTLINE_ELEMENTS`] path elements
/// (as VectorCraft leaves out the outlines of selections with too many anchors).
pub(super) fn copy_outline_paths(doc: &Document, n: &Node, outline: &BezPath) -> Vec<BezPath> {
    if !vectorcraft_render::picabia::draws_copies(doc, n) {
        return Vec::new();
    }
    let Some(out) = vectorcraft_modifiers::evaluate_node_in_doc(doc, n) else { return Vec::new() };
    let shown = out.instances.len().min(MAX_OUTLINES);
    if outline.elements().len().saturating_mul(shown) > MAX_OUTLINE_ELEMENTS {
        return Vec::new();
    }
    out.instances
        .iter()
        .filter(|i| i.transform != Affine::IDENTITY)
        .take(MAX_OUTLINES)
        .map(|i| {
            let mut bp = outline.clone();
            bp.apply_affine(i.transform);
            bp
        })
        .collect()
}

/// Stroke `outline` on each of `n`'s copies.
pub(super) fn copy_outlines(p: &egui::Painter, doc: &Document, n: &Node, outline: &BezPath, xf: &Xf, stroke: Stroke) {
    for bp in copy_outline_paths(doc, n, outline) {
        stroke_path(p, &bp, xf, stroke);
    }
}

/// The most path elements the stencil markers draw for one object (a fine lattice has many).
const MAX_MARKER_ELEMENTS: usize = 50_000;

/// What a selected object shows besides its own outline: a faint outline on its copies, and,
/// for a stencil with a computed sheet, its tie bars (thin, in the selection colour) and the
/// repeat cell its tile is cut from (dashed).
pub(super) fn selected_outlines(p: &egui::Painter, doc: &Document, n: &Node, outline: &BezPath, xf: &Xf, color: egui::Color32) {
    copy_outlines(p, doc, n, outline, xf, Stroke::new(1.0, color.gamma_multiply(0.4)));
    for (bp, dashed) in stencil_markers(doc, n) {
        if dashed {
            for pts in super::outline::to_screen_path(&bp, xf) {
                p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 4.0, 3.0));
            }
        } else {
            stroke_path(p, &bp, xf, Stroke::new(1.0, color.gamma_multiply(0.85)));
        }
    }
}

/// A stencil object's markers: its tie bars, and its repeat cell (dashed); none without a sheet,
/// or past [`MAX_MARKER_ELEMENTS`].
pub(super) fn stencil_markers(doc: &Document, n: &Node) -> Vec<(BezPath, bool)> {
    if !vectorcraft_engine::picabia_stencil::is_stencil(n) {
        return Vec::new();
    }
    let Some((sheet, _)) = vectorcraft_engine::picabia_stencil::lookup(doc, n) else { return Vec::new() };
    let mut out = Vec::new();
    if !sheet.info.ties.elements().is_empty() && sheet.info.ties.elements().len() <= MAX_MARKER_ELEMENTS {
        out.push((sheet.info.ties.clone(), false));
    }
    if let Some(c) = sheet.info.cell {
        let mut bp = BezPath::new();
        bp.move_to(c[0]);
        for q in &c[1..] {
            bp.line_to(*q);
        }
        bp.close_path();
        out.push((bp, true));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_color::Paint;
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn hovering_an_arrayed_object_outlines_every_copy() {
        let mut d = Document::new(400.0, 100.0);
        let l = d.layers[0].id;
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
        let outline = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)).to_bezpath();
        assert!(super::copy_outline_paths(&d, &n, &outline).is_empty(), "no stack: the object's own outline only");
        let src = r#"{"v":1,"stack":[{"id":"m1","kind":"linearArray","count":3,"offsetX":40}]}"#;
        vectorcraft_modifiers::set_node_source(&mut n, Some(src.into()));
        let id = d.insert(Some(l), 0, n).unwrap();
        let n = Arc::new(d.node(id).unwrap().clone());
        let copies = super::copy_outline_paths(&d, &n, &outline);
        let xs: Vec<f64> = copies.iter().map(|bp| vectorcraft_geom::Shape::bounding_box(bp).x0).collect();
        assert_eq!(xs, [40.0, 80.0], "the two copies besides the object");
    }
}
