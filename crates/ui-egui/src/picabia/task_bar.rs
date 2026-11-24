//! PICABIA (PIC3.4): the Contextual Task Bar's Modifiers button. `canvas::task_bar` shows it with
//! [`button`] in the bar, before the Fill chip, on every object selection that takes modifiers (not
//! with the Artboard tool); the bar lays itself out around it. It opens the grouped Add Modifier
//! menu (`super::add_menu`); when a selected object has modifiers the menu starts with Show
//! Modifiers (the Properties panel's section, opened and scrolled to), Apply Modifiers and Remove
//! All Modifiers.

use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Divider;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::access::PopupKind;
use egui::Ui;
use serde_json::json;

use crate::VectorcraftApp;

/// The Modifiers button and its menu, for a selection that takes modifiers (not on artboards).
pub fn button(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.tool_id() == "artboard" {
        return;
    }
    let Some((_, stacked)) = super::properties::offer(app, ui.ctx()) else { return };
    let opener = IconButton::new(IconSpec::glyph(&super::add_menu::ModifiersGlyph)).label(tl!("Modifiers")).popup(PopupKind::Menu).show(ui);
    Menu::new(tl!("Modifiers")).opened_by(&opener.response, Trigger::Press).show(ui, |ui, _| {
        if stacked {
            if MenuItem::new(tl!("Show Modifiers")).show(ui).ran {
                super::properties::reveal(app);
            }
            for (label, cmd) in [(tl!("Apply Modifiers"), "modifier.apply"), (tl!("Remove All Modifiers"), "modifier.remove_all")] {
                if MenuItem::new(label).show(ui).ran
                    && let Err(e) = app.run(cmd, json!({}))
                {
                    app.ui.status = e;
                }
            }
            Divider::horizontal().show(ui);
        }
        super::add_menu::items(app, ui);
    });
}
