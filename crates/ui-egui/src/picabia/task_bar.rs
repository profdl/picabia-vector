//! PICABIA (PIC3.4): the Contextual Task Bar's Modifiers button. `canvas::task_bar` adds its width
//! to the bar's chips with [`chip_widths`] (after Lock, so upstream's chips keep their places in
//! the layout) and draws it with [`button`] in the chips' row, before the Fill chip: the chips,
//! Recolor Artwork and Lock keep their order, and the bar's layout (its width, rows and centring
//! under the selection) counts the button. It opens the grouped Add Modifier menu
//! (`super::add_menu`); when a selected object has modifiers the menu starts with Show Modifiers
//! (the Properties panel's section, opened and scrolled to), Apply Modifiers and Remove All
//! Modifiers.

use egui::{Sense, Ui, vec2};
use serde_json::json;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The button's side, as the bar's other icon buttons.
const SIZE: f32 = 30.0;

/// The bar's chip widths `widths`, with the button's after them for a selection that takes
/// modifiers (not with the Artboard tool), when the bar fits with it: when the button would add a
/// row, move an action or leave anything out (`layout` sums up the bar's layout for given chip
/// widths), it is left out (Properties and Object ▸ Modifiers still have the menu).
pub fn chip_widths<T: PartialEq>(
    app: &VectorcraftApp,
    ctx: &egui::Context,
    on_artboards: bool,
    mut widths: Vec<f32>,
    layout: impl Fn(&[f32]) -> Option<T>,
) -> Vec<f32> {
    if on_artboards || super::properties::offer(app, ctx).is_none() {
        return widths;
    }
    let without = layout(&widths);
    widths.push(SIZE);
    if without.is_none() || layout(&widths) != without {
        widths.pop();
    }
    widths
}

/// The Modifiers button and its menu, when `here` (the bar's layout put the button in this row:
/// the selection takes modifiers and the bar fits with it).
pub fn button(app: &mut VectorcraftApp, ui: &mut Ui, here: bool) {
    if !here {
        return;
    }
    let Some((_, stacked)) = super::properties::offer(app, ui.ctx()) else { return };
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.hover);
    }
    let bg = if resp.hovered() { t.hover } else { t.panel };
    super::add_menu::paint_glyph(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), t.icon, bg);
    let resp = resp.on_hover_text(tl!("Modifiers"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Modifiers")));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(180.0);
        if stacked {
            if ui.button(tl!("Show Modifiers")).clicked() {
                super::properties::reveal(app);
                ui.close();
            }
            for (label, cmd) in [(tl!("Apply Modifiers"), "modifier.apply"), (tl!("Remove All Modifiers"), "modifier.remove_all")] {
                if ui.button(label).clicked() {
                    if let Err(e) = app.run(cmd, json!({})) {
                        app.ui.status = e;
                    }
                    ui.close();
                }
            }
            ui.separator();
        }
        super::add_menu::items(app, ui);
    });
}
