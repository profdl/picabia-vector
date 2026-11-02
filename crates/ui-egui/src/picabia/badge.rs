//! PICABIA (PIC3.4): the modifier badge on the canvas. When one to [`MAX_BADGES`] selected
//! objects have a modifier stack, each gets a small tag just outside its bounding box's top-right
//! corner: the modifiers glyph and the number of rows, with the tooltip "3 modifiers — click to
//! show". A click opens the Properties panel's Modifiers section (`super::properties::reveal`).
//!
//! The badge is drawn in screen points (it scales with the UI, not the zoom), in the theme's
//! colours, 8 points out from the corner on the diagonal, so it never covers the bounding box's
//! handles. It shows with the selection tools (as the task bar does), not while something is
//! dragged, and only where it fits inside the canvas. Drawn each frame from `picabia_ui::show`
//! (no seam of its own).

use egui::{Pos2, Rect, Sense, Stroke, pos2, vec2};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The most stacked selected objects that get badges; with more, none do.
pub const MAX_BADGES: usize = 8;
/// How far the badge sits from the bounding box's corner, along the diagonal (screen points).
const GAP: f32 = 8.0;
const HEIGHT: f32 = 18.0;

/// The badges' places kept for (document, revision, selection): each stacked object, its visual
/// bounds and its number of rows.
type Kept = (u64, u64, Vec<NodeId>, Vec<(NodeId, vectorcraft_geom::Rect, usize)>);

/// The stacked selected objects with their bounds and rows (none past [`MAX_BADGES`]), kept until
/// the document or the selection changes.
fn stacked(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<(NodeId, vectorcraft_geom::Rect, usize)> {
    let Some(st) = app.session.active() else { return vec![] };
    let key = egui::Id::unique("picabia-modifier-badges");
    let ids = st.selection.objects.clone();
    if let Some((uid, rev, kept_ids, v)) = ctx.data(|d| d.get_temp::<Kept>(key))
        && uid == st.uid
        && rev == st.revision
        && kept_ids == ids
    {
        return v;
    }
    let mut v = vec![];
    for id in &ids {
        let Some(n) = st.doc.node(*id) else { continue };
        let rows = match vectorcraft_modifiers::node_stack(n) {
            Some(Ok(s)) if !s.modifiers.is_empty() => s.modifiers.len(),
            _ => continue,
        };
        let Some(b) = st.doc.bounds_of(&[*id], true) else { continue };
        v.push((*id, b, rows));
        if v.len() > MAX_BADGES {
            v.clear();
            break;
        }
    }
    ctx.data_mut(|d| d.insert_temp::<Kept>(key, (st.uid, st.revision, ids, v.clone())));
    v
}

/// Where the badge for bounds whose screen corners are `quad` goes: its rect, just outside the
/// corner that is furthest up and right on screen (the view may be rotated), `width` wide.
pub fn place(quad: &[Pos2], width: f32) -> Option<Rect> {
    let corner = quad.iter().copied().filter(|p| p.x.is_finite() && p.y.is_finite()).max_by(|a, b| (a.x - a.y).total_cmp(&(b.x - b.y)))?;
    Some(Rect::from_min_size(pos2(corner.x + GAP, corner.y - GAP - HEIGHT), vec2(width, HEIGHT)))
}

/// The badges, for the stacked selected objects (see the module).
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (Some(canvas), Some(view)) = (app.canvas_rect, app.view()) else { return };
    if !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection") || ctx.input(|i| i.pointer.is_decidedly_dragging()) {
        return;
    }
    let xf = crate::canvas::Xf::new(canvas, view);
    let badges = stacked(app, ctx);
    let t = Tokens::get(ctx);
    let mut clicked = false;
    for (i, (_, b, rows)) in badges.iter().enumerate() {
        let text = rows.to_string();
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), egui::FontId::proportional(11.5), t.text_strong));
        let width = 6.0 + 12.0 + 4.0 + galley.size().x + 6.0;
        let Some(r) = place(&xf.quad(*b), width).filter(|r| canvas.contains_rect(*r)) else { continue };
        let tip = crate::i18n::trn(crate::i18n::current(), *rows as u64, "{n} modifier — click to show", "{n} modifiers — click to show");
        egui::Area::new(egui::Id::unique(("picabia-modifier-badge", i))).order(egui::Order::Middle).fixed_pos(r.min).constrain(false).show(
            ctx,
            |ui| {
                let (rect, resp) = ui.allocate_exact_size(r.size(), Sense::click());
                let fill = if resp.hovered() { t.hover } else { t.panel };
                ui.painter().rect_filled(rect, 4.0, fill);
                ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, t.selection), egui::StrokeKind::Inside);
                let glyph = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 6.0), vec2(12.0, 12.0));
                super::add_menu::paint_glyph(ui.painter(), glyph, t.icon, fill);
                ui.painter().galley(pos2(glyph.right() + 4.0, rect.center().y - galley.size().y / 2.0), galley, t.text_strong);
                let resp = resp.on_hover_text(tip.as_str()).on_hover_cursor(egui::CursorIcon::PointingHand);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip.as_str()));
                clicked |= resp.clicked();
            },
        );
    }
    if clicked {
        super::properties::reveal(app);
    }
}
