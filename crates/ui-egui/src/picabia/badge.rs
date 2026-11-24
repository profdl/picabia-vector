//! PICABIA (PIC3.4): the modifier badge on the canvas. When one to [`MAX_BADGES`] selected
//! objects have a modifier stack, each gets a small tag just outside its bounding box's top-right
//! corner: the modifiers glyph and the number of rows, with the tooltip "3 modifiers — click to
//! show". A click opens the Properties panel's Modifiers section (`super::properties::reveal`).
//!
//! The badge is a button of the framework's on the canvas, in screen points (it scales with the UI,
//! not the zoom), 8 points out from the corner on the diagonal, outside the box, so it never covers
//! the bounding box's handles or the modifier handles inside it. It shows when the Contextual Task
//! Bar may (seam in `canvas::show_body` beside the bar's: not on the home screen, in Presentation
//! Mode, while a 3D revolve is edited, while the Simplify or Smooth slider shows, nor while
//! something is dragged), with the selection tools, and only where it fits inside the canvas.

use craft::ui::buttons::Button;
use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::{Props as _, Show as _};
use craft::ui::primitives::IconSpec;
use craft::ui::style::Variant;
use egui::{Pos2, Rect, pos2, vec2};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;

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

/// Whether the badges may show: with the selection tools, off the home screen.
fn allowed(app: &VectorcraftApp) -> bool {
    matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection") && !crate::menus::home_showing(app)
}

/// The badges on the canvas `space`, for the stacked selected objects (see the module).
pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace) {
    if !allowed(app) {
        return;
    }
    let (xf, canvas) = (space.view, space.view.rect);
    let badges = stacked(app, ui.ctx());
    let mut clicked = false;
    for (i, (_, b, rows)) in badges.iter().enumerate() {
        let text = rows.to_string();
        let width = 6.0 + 12.0 + 4.0 + 8.0 * text.len() as f32 + 6.0;
        let Some(r) = place(&xf.quad(*b), width).filter(|r| canvas.contains_rect(*r)) else { continue };
        let tip = crate::i18n::trn(crate::i18n::current(), *rows as u64, "{n} modifier — click to show", "{n} modifiers — click to show");
        let shown = Button::new(text)
            .icon(IconSpec::glyph(&super::add_menu::ModifiersGlyph))
            .variant(Variant::SECONDARY)
            .tooltip(tip)
            .id_salt(("picabia-modifier-badge", i))
            .show_at(ui, r);
        clicked |= shown.clicked();
    }
    if clicked {
        super::properties::reveal(app);
    }
}
