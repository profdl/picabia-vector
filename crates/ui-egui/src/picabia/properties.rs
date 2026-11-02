//! PICABIA (PIC3.4): the Properties panel's Modifiers section, between Appearance and Align
//! (`panels::properties::show` calls [`section`]), so modifiers are found from the panel that is
//! always open in the Essentials workspace.
//!
//! The section is the selection's: shown while objects (not only layers) are selected. Its header
//! looks like the panel's other section headers, with the number of rows and a chevron; a click
//! collapses it (kept for the session in `PicabiaUi::modifiers_collapsed`). Open, it shows the
//! Appearance panel's modifier rows for the selection (`super::appearance::modifier_rows`: eye,
//! kind and copies, the row's fields when its disclosure is open, the `⋯` menu, the copy count and
//! Apply; one "Mixed" row when several objects' stacks differ), then "+ Add Modifier", which opens
//! the grouped Add Modifier menu (`super::add_menu`).

use egui::{Sense, Ui, pos2, vec2};

use crate::theme::Tokens;
use crate::widgets::{self, divider};
use crate::{VectorcraftApp, icons};

/// The selection as a cache key: the document, its revision and the selected objects (their
/// number and a hash of their ids), without copying the selection.
pub(crate) fn selection_key(app: &VectorcraftApp) -> Option<(u64, u64, usize, u64)> {
    use std::hash::{Hash, Hasher};
    let st = app.session.active()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    st.selection.objects.iter().for_each(|id| id.0.hash(&mut h));
    Some((st.uid, st.revision, st.selection.objects.len(), h.finish()))
}

/// What the selection offers: `None` when the section and the task bar button don't show (nothing
/// selected, or only layers), else the first selected object's number of modifier rows and whether
/// any selected object has modifier data.
pub type Offer = Option<(usize, bool)>;

/// The selection's [`Offer`], worked out once per document revision and selection (the Properties
/// section and the task bar both ask every frame).
pub fn offer(app: &VectorcraftApp, ctx: &egui::Context) -> Offer {
    type Kept = ((u64, u64, usize, u64), Offer);
    let key = selection_key(app)?;
    let id = egui::Id::unique("picabia-modifier-offer");
    if let Some((k, o)) = ctx.data(|d| d.get_temp::<Kept>(id))
        && k == key
    {
        return o;
    }
    let o = compute(app);
    ctx.data_mut(|d| d.insert_temp::<Kept>(id, (key, o)));
    o
}

fn compute(app: &VectorcraftApp) -> Offer {
    let st = app.session.active()?;
    let mut objects = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).filter(|n| !n.is_layer());
    let first = objects.next()?;
    let rows = match vectorcraft_modifiers::node_stack(first) {
        Some(Ok(s)) => s.modifiers.len(),
        _ => 0,
    };
    let stacked = vectorcraft_modifiers::has_modifiers(first) || objects.any(vectorcraft_modifiers::has_modifiers);
    Some((rows, stacked))
}

/// Show the Properties panel, leaving it be when it already shows (docked, as a flyout or
/// floating: `panel_docking::reveal_panel` never puts it away, as `window.panel` would), with the
/// Modifiers section open and scrolled into view on its next frame (the canvas badge, the task
/// bar's Show Modifiers).
pub fn reveal(app: &mut VectorcraftApp) {
    app.picabia.modifiers_collapsed = false;
    match crate::panel_docking::reveal_panel(app, "properties") {
        Ok(_) => app.picabia.reveal_modifiers = true,
        Err(e) => {
            app.picabia.reveal_modifiers = false;
            app.ui.status = e;
        }
    }
}

/// The Modifiers section and the divider under it, for a selection that can take modifiers.
pub fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((count, _)) = offer(app, ui.ctx()) else { return };
    let header = header(ui, count, !app.picabia.modifiers_collapsed);
    if header.clicked() {
        app.picabia.modifiers_collapsed = !app.picabia.modifiers_collapsed;
    }
    if app.picabia.reveal_modifiers {
        app.picabia.reveal_modifiers = false;
        app.picabia.modifiers_collapsed = false;
        header.scroll_to_me(Some(egui::Align::TOP));
    }
    if !app.picabia.modifiers_collapsed {
        super::appearance::modifier_rows(app, ui);
        ui.add_space(4.0);
        let add = widgets::flat_button(ui, tl!("+ Add Modifier"), ui.available_width()).on_hover_text(tl!("Add a modifier to the selected objects"));
        egui::Popup::menu(&add).show(|ui| {
            ui.set_min_width(180.0);
            super::add_menu::items(app, ui);
        });
    }
    divider(ui);
}

/// The header: "Modifiers" as the panel's section headers show it, the number of rows (dim) and a
/// chevron at the right; a click on it toggles the section.
fn header(ui: &mut Ui, count: usize, open: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
    let painter = ui.painter();
    let name = tl!("Modifiers");
    painter.text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text_strong);
    let chevron = egui::Rect::from_center_size(pos2(rect.right() - 7.0, rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, chevron, if resp.hovered() { t.text_strong } else { t.icon });
    if count > 0 {
        painter.text(
            pos2(chevron.left() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(11.5),
            t.text_dim,
        );
    }
    ui.add_space(2.0);
    let tip = if open { tl!("Hide the modifiers") } else { tl!("Show the modifiers") };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, name));
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}
