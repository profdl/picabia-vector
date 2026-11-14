//! PICABIA (PIC3.4): the modifier badge on the canvas. When one to [`MAX_BADGES`] selected
//! objects have a modifier stack, each gets a small tag just outside its bounding box's top-right
//! corner: the modifiers glyph and the number of rows, with the tooltip "3 modifiers — click to
//! show". A click opens the Properties panel's Modifiers section (`super::properties::reveal`).
//!
//! The badge is drawn in screen points (it scales with the UI, not the zoom), in the theme's
//! colours, 8 points out from the corner on the diagonal, outside the box, so it never covers the
//! bounding box's handles or the modifier handles inside it. It shows when the Contextual Task Bar
//! may (`canvas::show`: not on the home screen, in Presentation Mode, while a 3D revolve is edited,
//! while the Simplify or Smooth slider shows, nor while something is dragged), with the selection
//! tools, only where it fits inside the canvas, and not where a floating panel, window or menu lies
//! over it (it would otherwise draw over them and take their clicks). Drawn each frame from
//! `picabia_ui::show` (no seam of its own).

use egui::{Pos2, Rect, Sense, Stroke, pos2, vec2};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The most stacked selected objects that get badges; with more, none do.
pub const MAX_BADGES: usize = 8;
/// How far the badge sits from the bounding box's corner, along the diagonal (screen points).
const GAP: f32 = 8.0;
const HEIGHT: f32 = 18.0;

/// One badge's object, its visual bounds and its number of rows.
type Badge = (NodeId, vectorcraft_geom::Rect, usize);

/// The stacked selected objects with their bounds and rows (none past [`MAX_BADGES`]), kept until
/// the document or the selection changes (keyed without copying the selection).
fn stacked(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<Badge> {
    type Kept = ((u64, u64, usize, u64), Vec<Badge>);
    let (Some(st), Some(key)) = (app.session.active(), super::properties::selection_key(app)) else { return vec![] };
    let id = egui::Id::unique("picabia-modifier-badges");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<Kept>(id))
        && k == key
    {
        return v;
    }
    let mut v = vec![];
    for id in &st.selection.objects {
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
    ctx.data_mut(|d| d.insert_temp::<Kept>(id, (key, v.clone())));
    v
}

/// Where the badge for bounds whose screen corners are `quad` goes: its rect, just outside the
/// corner that is furthest up and right on screen (the view may be rotated), `width` wide.
pub fn place(quad: &[Pos2], width: f32) -> Option<Rect> {
    let corner = quad.iter().copied().filter(|p| p.x.is_finite() && p.y.is_finite()).max_by(|a, b| (a.x - a.y).total_cmp(&(b.x - b.y)))?;
    Some(Rect::from_min_size(pos2(corner.x + GAP, corner.y - GAP - HEIGHT), vec2(width, HEIGHT)))
}

/// Badge `i`'s area.
pub fn area_id(i: usize) -> egui::Id {
    egui::Id::unique(("picabia-modifier-badge", i))
}

/// Whether the task bar's conditions allow the badges (`canvas::show` draws the bar under them).
fn allowed(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection")
        && !crate::menus::home_showing(app)
        && app.ui.screen_mode < 3
        && !app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active)
        && !crate::dialogs::simplify_slider::showing(app)
        && !crate::dialogs::smooth_slider::showing(app)
        && !ctx.input(|i| i.pointer.is_decidedly_dragging())
}

/// Whether something drawn over the canvas (a floating panel or window, a menu or popup; not
/// tooltips nor the badges themselves) lay over `r` last frame.
fn covered(ctx: &egui::Context, r: Rect, badges: usize) -> bool {
    let own: Vec<egui::Id> = (0..=MAX_BADGES.max(badges)).map(area_id).collect();
    ctx.memory(|m| {
        m.areas().visible_layer_ids().into_iter().any(|l| {
            !matches!(l.order, egui::Order::Background | egui::Order::Tooltip | egui::Order::Debug)
                && !own.contains(&l.id)
                && m.area_rect(l.id).is_some_and(|a| a.intersects(r))
        })
    })
}

/// The badges, for the stacked selected objects (see the module).
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (Some(canvas), Some(view)) = (app.canvas_rect, app.view()) else { return };
    if !allowed(app, ctx) {
        return;
    }
    let xf = crate::canvas::Xf::at(canvas, view, app.canvas_ui_zoom);
    let badges = stacked(app, ctx);
    let t = Tokens::get(ctx);
    let mut clicked = false;
    for (i, (_, b, rows)) in badges.iter().enumerate() {
        let text = rows.to_string();
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text, egui::FontId::proportional(11.5), t.text_strong));
        let width = 6.0 + 12.0 + 4.0 + galley.size().x + 6.0;
        let Some(r) = place(&xf.quad(*b), width).filter(|r| canvas.contains_rect(*r)) else { continue };
        if covered(ctx, r, badges.len()) {
            continue;
        }
        let tip = crate::i18n::trn(crate::i18n::current(), *rows as u64, "{n} modifier — click to show", "{n} modifiers — click to show");
        egui::Area::new(area_id(i)).order(egui::Order::Middle).fixed_pos(r.min).constrain(false).show(ctx, |ui| {
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
        });
    }
    if clicked {
        super::properties::reveal(app);
    }
}
