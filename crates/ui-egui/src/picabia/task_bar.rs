//! PICABIA (PIC3.4): the Contextual Task Bar's Modifiers button (`canvas::task_bar` calls
//! [`button`] after its labelled buttons, before the Fill chip, so the chips, Recolor Artwork and
//! Lock keep their places from the right). It
//! opens the grouped Add Modifier menu (`super::add_menu`); when a selected object has modifiers
//! the menu starts with Show Modifiers (the Properties panel's section, opened and scrolled to),
//! Apply Modifiers and Remove All Modifiers.

use egui::{Sense, Ui, vec2};
use serde_json::json;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The button's side, as the bar's other icon buttons.
const SIZE: f32 = 30.0;
/// The bar's gap between items.
const GAP: f32 = 6.0;
/// The bar keeps this far inside the canvas (`canvas::task_bar`).
const MARGIN: f32 = 8.0;

/// Whether the bar, `bar` wide last frame (with the button when `shown`), still fits the canvas,
/// `canvas` wide, with the button: when it doesn't, the button is left out (Properties and Object ▸
/// Modifiers still have the menu). Measured without the button, so it doesn't flicker.
pub fn fits(bar: f32, shown: bool, canvas: f32) -> bool {
    let without = bar - if shown { SIZE + GAP } else { 0.0 };
    without + SIZE + GAP <= canvas - 2.0 * MARGIN
}

/// The Modifiers button and its menu, for a selection that can take modifiers, when the bar fits.
pub fn button(app: &mut VectorcraftApp, ui: &mut Ui) {
    if super::properties::eligible(app).is_none() {
        return;
    }
    let shown_id = egui::Id::unique("picabia-task-bar-modifiers");
    let shown = ui.ctx().data(|d| d.get_temp::<bool>(shown_id)).unwrap_or(false);
    let fit = match (crate::canvas::task_bar_rect(ui.ctx()), app.canvas_rect) {
        (Some(bar), Some(canvas)) => fits(bar.width(), shown, canvas.width()),
        _ => true,
    };
    ui.ctx().data_mut(|d| d.insert_temp(shown_id, fit));
    if !fit {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.hover);
    }
    let bg = if resp.hovered() { t.hover } else { t.panel };
    super::add_menu::paint_glyph(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), t.icon, bg);
    let resp = resp.on_hover_text(tl!("Modifiers"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Modifiers")));
    let stacked = super::properties::has_stack(app);
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
