//! PICABIA: outlines on an object's modifier copies: the hover highlight on every copy, and a
//! faint outline on a selected object's copies when View › Show Modifier Copy Outlines is on (the
//! selection box stays on the object itself); a
//! selected stencil's tie bars and repeat cell; the selected object's modifier handles.

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Pos2, Stroke, vec2};
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, BezPath};
use vectorcraft_modifiers::Guide;

use super::Xf;
use super::outline::stroke_path;
use crate::VectorcraftApp;
use crate::theme::Tokens;

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
    if out.has_geometry() {
        return shaped_outlines(doc, n);
    }
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

/// The paths of `n`'s copies as drawn, when geometry rows reshape them (a transform of the
/// object's outline isn't what is drawn): every copy, the unmoved one too, within the same caps.
fn shaped_outlines(doc: &Document, n: &Node) -> Vec<BezPath> {
    fn walk(n: &Node, depth: usize, budget: &mut usize, out: &mut Vec<BezPath>) {
        if depth > vectorcraft_doc::MAX_DEPTH || out.len() >= MAX_OUTLINES {
            return;
        }
        if let Some(p) = n.path_data() {
            let bp = p.to_bezpath();
            let k = bp.elements().len();
            if k <= *budget {
                *budget -= k;
                out.push(bp);
            }
            return;
        }
        for c in n.children().into_iter().flatten() {
            walk(c, depth + 1, budget, out);
        }
    }
    let Some(art) = vectorcraft_render::picabia::art(doc, n) else { return Vec::new() };
    let (mut out, mut budget) = (Vec::new(), MAX_OUTLINE_ELEMENTS);
    walk(&art, 0, &mut budget, &mut out);
    out
}

/// Stroke `outline` on each of `n`'s copies.
pub(super) fn copy_outlines(p: &egui::Painter, doc: &Document, n: &Node, outline: &BezPath, xf: &Xf, stroke: Stroke) {
    for bp in copy_outline_paths(doc, n, outline) {
        stroke_path(p, &bp, xf, stroke);
    }
}

/// The most path elements the stencil markers draw for one object (a fine lattice has many).
const MAX_MARKER_ELEMENTS: usize = 50_000;

/// What a selected object shows besides its own outline: a faint outline on its copies when
/// `copies` (View › Show Modifier Copy Outlines), and, for a stencil with a computed sheet, its
/// tie bars (thin, in the selection colour) and the repeat cell its tile is cut from (dashed).
pub(super) fn selected_outlines(p: &egui::Painter, doc: &Document, n: &Node, outline: &BezPath, xf: &Xf, color: egui::Color32, copies: bool) {
    if copies {
        copy_outlines(p, doc, n, outline, xf, Stroke::new(1.0, color.gamma_multiply(0.4)));
    }
    for (bp, dashed) in stencil_markers(doc, n).iter() {
        if *dashed {
            for pts in super::outline::to_screen_path(bp, xf) {
                p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 4.0, 3.0));
            }
        } else {
            stroke_path(p, bp, xf, Stroke::new(1.0, color.gamma_multiply(0.85)));
        }
    }
}

/// How many sheets' markers are kept (one per selected stencil, a few selected at once).
const KEPT_MARKERS: usize = 8;

type Markers = Arc<Vec<(BezPath, bool)>>;

/// Markers kept per sheet, so a selected stencil's ties aren't copied every frame.
static MARKERS: Mutex<Vec<(Arc<vectorcraft_engine::picabia_stencil::Sheet>, Markers)>> = Mutex::new(Vec::new());

/// A stencil object's markers: its tie bars, and its repeat cell (dashed); none without a sheet,
/// or past [`MAX_MARKER_ELEMENTS`]. Kept per sheet.
pub(super) fn stencil_markers(doc: &Document, n: &Node) -> Markers {
    if !vectorcraft_engine::picabia_stencil::is_stencil(n) {
        return Markers::default();
    }
    let Some((sheet, _)) = vectorcraft_engine::picabia_stencil::lookup(doc, n) else { return Markers::default() };
    let mut kept = MARKERS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = kept.iter().position(|(s, _)| Arc::ptr_eq(s, &sheet)) {
        let hit = kept.remove(at);
        let m = hit.1.clone();
        kept.insert(0, hit);
        return m;
    }
    let m = Arc::new(markers_of(&sheet));
    kept.insert(0, (sheet, m.clone()));
    kept.truncate(KEPT_MARKERS);
    m
}

fn markers_of(sheet: &vectorcraft_engine::picabia_stencil::Sheet) -> Vec<(BezPath, bool)> {
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

/// Handle radius (screen points), and under the pointer or dragged.
pub(super) const KNOB: f32 = 4.5;
const KNOB_HOT: f32 = 6.0;
/// The most points a guide circle is drawn with.
const CIRCLE_STEPS: usize = 96;

/// The selected object's modifier handles (the engine decides which rows show them,
/// `Session::picabia_mod_handles`): dashed guides, round knobs ringed in the selection colour
/// (greyed when a field they set is bound), larger under the pointer, and the readout of the one
/// dragged or hovered. Every size is in screen points, so the handles keep their size at any zoom.
pub(super) fn modifier_handles(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf, view: vectorcraft_engine::ViewInfo) {
    let Some(v) = app.session.picabia_mod_handles(view) else { return };
    let t = Tokens::get(p.ctx());
    let guide = Stroke::new(1.0, t.selection.gamma_multiply(0.8));
    for g in &v.guides {
        match g {
            Guide::Line { from, to } => {
                let pts = [xf.to_screen(*from), xf.to_screen(*to)];
                if pts.iter().all(|q| q.is_finite()) {
                    p.extend(egui::Shape::dashed_line(&pts, guide, 5.0, 4.0));
                }
            }
            Guide::Circle { center, radius } => {
                let r = radius * xf.zoom;
                if !(r.is_finite() && r > 0.5) {
                    continue;
                }
                let pts: Vec<Pos2> = (0..=CIRCLE_STEPS)
                    .map(|k| {
                        let a = k as f64 / CIRCLE_STEPS as f64 * std::f64::consts::TAU;
                        xf.to_screen(*center + vectorcraft_geom::Vec2::new(a.cos(), a.sin()) * *radius)
                    })
                    .collect();
                if pts.iter().all(|q| q.is_finite()) {
                    p.extend(egui::Shape::dashed_line(&pts, guide, 5.0, 4.0));
                }
            }
            Guide::Point(q) => {
                let c = xf.to_screen(*q);
                if c.is_finite() {
                    p.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], guide);
                    p.line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], guide);
                }
            }
        }
    }
    for h in &v.handles {
        let c = xf.to_screen(h.at);
        if !c.is_finite() {
            continue;
        }
        let r = if h.hot { KNOB_HOT } else { KNOB };
        let (fill, ring) = if h.bound { (t.panel, t.text_disabled) } else { (t.measure_bg, t.selection) };
        p.circle_filled(c, r, fill);
        p.circle_stroke(c, r, Stroke::new(2.0, ring));
    }
    if let Some((at, text)) = &v.readout {
        let c = xf.to_screen(*at);
        if c.is_finite() {
            let galley = p.layout(text.clone(), egui::FontId::proportional(11.0), t.text, 260.0);
            let sp = c + vec2(12.0, -12.0 - galley.size().y);
            let r = egui::Rect::from_min_size(sp, galley.size() + vec2(12.0, 8.0));
            p.rect_filled(r, egui::CornerRadius::same(3), t.panel);
            p.rect_stroke(r, egui::CornerRadius::same(3), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            p.galley(sp + vec2(6.0, 4.0), galley, t.text);
        }
    }
}

#[cfg(test)]
#[path = "picabia_tests.rs"]
mod handle_tests;

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

    /// A stencil sheet computed since the frame shown renders the whole frame again, whatever
    /// placed documents' bitmaps did: the stencil counter is its own key field, never read as a
    /// placed-bitmap generation (where a range with no bitmaps made in it repaints nothing).
    #[test]
    fn a_stencil_sheet_computed_since_the_frame_shown_renders_the_whole_frame() {
        use serde_json::json;
        use vectorcraft_geom::Affine;

        use crate::VectorcraftApp;
        use crate::caches::CacheKey;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        app.canvas.worker_started = true; // every frame on this thread
        app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 60, "height": 40})).unwrap();
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| super::super::show(&mut app, ui));
        out.textures_delta.clear();
        let key = app.canvas.key.clone().unwrap();
        // (Not compared with the process-wide `generation()`: tests running beside this one move it.)
        let (_, opts) = app.canvas.shown.clone().unwrap();
        let view = Affine::translate((f64::from(key.w) / 2.0, f64::from(key.h) / 2.0))
            * Affine::rotate(key.rot.to_radians())
            * Affine::scale(key.zoom * f64::from(key.ppp))
            * Affine::translate((-key.cx, -key.cy));
        let doc = app.session.active().unwrap().doc.clone();
        let mut changed = |k: &CacheKey| super::super::raster::changed_pixels(&mut app, k, &doc, &opts, view, k.w, k.h);
        assert_eq!(changed(&key), Some(None), "nothing changed: nothing to render again");
        let sheet = CacheKey { stencil: key.stencil.wrapping_add(1), ..key.clone() };
        assert_eq!(changed(&sheet), None, "a new sheet: the whole frame");
        // The bug this guards: the stencil counter folded into `placed`, read as a range of
        // placed bitmaps made (none were), so nothing rendered again.
        assert_eq!(sheet.placed, key.placed);
    }
}
