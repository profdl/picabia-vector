//! PICABIA: outlines on an object's modifier copies: the hover highlight on every copy, and a
//! faint outline on a selected object's copies (the selection box stays on the object itself).

use egui::Stroke;
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, BezPath};

use super::Xf;
use super::outline::stroke_path;

/// `outline` (the object's own) moved to each of `n`'s copies but the unmoved one.
pub(super) fn copy_outline_paths(doc: &Document, n: &Node, outline: &BezPath) -> Vec<BezPath> {
    if !vectorcraft_render::picabia::draws_copies(doc, n) {
        return Vec::new();
    }
    let Some(out) = vectorcraft_modifiers::evaluate_node_in_doc(doc, n) else { return Vec::new() };
    out.instances
        .iter()
        .filter(|i| i.transform != Affine::IDENTITY)
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
