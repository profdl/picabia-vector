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

/// What the section shows for the selection: `None` when it is not shown (nothing selected, or
/// only layers), else the first selected object's number of modifier rows.
pub fn eligible(app: &VectorcraftApp) -> Option<usize> {
    let st = app.session.active()?;
    let mut objects = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).filter(|n| !n.is_layer());
    let first = objects.next()?;
    Some(match vectorcraft_modifiers::node_stack(first) {
        Some(Ok(s)) => s.modifiers.len(),
        _ => 0,
    })
}

/// Whether any selected object has modifier data.
pub fn has_stack(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).any(vectorcraft_modifiers::has_modifiers))
}

/// Open the Properties panel with the Modifiers section open and scrolled into view (the canvas
/// badge, the task bar's Show Modifiers).
pub fn reveal(app: &mut VectorcraftApp) {
    app.picabia.modifiers_collapsed = false;
    app.picabia.reveal_modifiers = true;
    if let Err(e) = app.run("window.panel", serde_json::json!({"panel": "properties"})) {
        app.ui.status = e;
    }
}

/// The Modifiers section and the divider under it, for a selection that can take modifiers.
pub fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(count) = eligible(app) else { return };
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
