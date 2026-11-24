//! PICABIA (PIC3.4): the Properties panel's Modifiers section, between Appearance and Align
//! (`panels::properties::show` calls [`section`]), so modifiers are found from the panel that is
//! always open in the Essentials workspace.
//!
//! The section is the selection's: shown while objects (not only layers) are selected. It is one of
//! the panel's sections, titled with the number of rows; its disclosure collapses it (kept for the session in `PicabiaUi::modifiers_collapsed`). Open, it shows the
//! Appearance panel's modifier rows for the selection (`super::appearance::modifier_rows`: eye,
//! kind and copies, the row's fields when its disclosure is open, the `⋯` menu, the copy count and
//! Apply; one "Mixed" row when several objects' stacks differ), then "+ Add Modifier", which opens
//! the grouped Add Modifier menu (`super::add_menu`).

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::menu::Menu;
use craft::ui::sections::Section;
use craft::ui::style::Variant;
use egui::Ui;

use crate::VectorcraftApp;

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

/// The Modifiers section, a section of the panel's (its title with the number of rows; its
/// disclosure collapses it), for a selection that can take modifiers.
pub fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((count, _)) = offer(app, ui.ctx()) else { return };
    let reveal = std::mem::take(&mut app.picabia.reveal_modifiers);
    let mut open = reveal || !app.picabia.modifiers_collapsed;
    let title = if count > 0 { format!("{} ({count})", tl!("Modifiers")) } else { tl!("Modifiers").to_string() };
    let shown = Section::new(title).open(Bind::bound(&mut open)).show(ui, |ui, _| {
        super::appearance::modifier_rows(app, ui);
        // The physics row, as the Appearance panel shows it: Add Physics is on this section's
        // menu, so its result shows here.
        super::physics::section(app, ui);
        let add = Button::new(tl!("+ Add Modifier")).variant(Variant::SECONDARY).tooltip(tl!("Add a modifier to the selected objects")).show(ui);
        Menu::new(tl!("Add Modifier")).opened_by(&add.response, Trigger::Press).show(ui, |ui, _| super::add_menu::items(app, ui));
    });
    app.picabia.modifiers_collapsed = !open;
    if reveal {
        shown.response.scroll_to_me(Some(egui::Align::TOP));
    }
}
