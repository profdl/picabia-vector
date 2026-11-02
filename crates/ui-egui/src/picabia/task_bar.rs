//! PICABIA (PIC3.4): the Contextual Task Bar's Modifiers button (`canvas::task_bar` calls
//! [`button`] after its labelled buttons, before the Fill chip: the chips, Recolor Artwork and
//! Lock keep their order, and the bar is 36 points wider for object selections; [`est_width`]
//! adds that to the bar's estimated width so it stays centred under the selection). It opens the
//! grouped Add Modifier menu (`super::add_menu`); when a selected object has modifiers the menu
//! starts with Show Modifiers (the Properties panel's section, opened and scrolled to), Apply
//! Modifiers and Remove All Modifiers.

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
/// The most the bar draws after the button: the Fill and Stroke chips (26 each), Recolor Artwork,
/// Lock or Shape Properties and More Options (30 each), their gaps, and the frame's margins.
const AFTER: f32 = 2.0 * 26.0 + 3.0 * 30.0 + 5.0 * GAP + 2.0 * 8.0 + 2.0;

/// Whether the bar fits the canvas, `canvas` wide, with the button, when what it drew before the
/// button this frame is `before` wide: when it doesn't, the button is left out (Properties and
/// Object ▸ Modifiers still have the menu).
pub fn fits(before: f32, canvas: f32) -> bool {
    before + GAP + SIZE + GAP + AFTER <= canvas - 2.0 * MARGIN
}

/// Where the last frame's decision is kept, for [`est_width`].
fn shown_id() -> egui::Id {
    egui::Id::unique("picabia-task-bar-modifiers")
}

/// What the button adds to the bar's estimated width (`canvas::task_bar` centres the bar with
/// it): its side and gap for a selection that takes modifiers, unless it didn't fit last frame.
pub fn est_width(app: &VectorcraftApp, ctx: &egui::Context) -> f32 {
    let shown = ctx.data(|d| d.get_temp::<bool>(shown_id())).unwrap_or(true);
    let objects = app.session.tool_id() != "artboard" && super::properties::offer(app, ctx).is_some();
    if shown && objects { SIZE + GAP } else { 0.0 }
}

/// The Modifiers button and its menu, for a selection that can take modifiers, when the bar fits.
pub fn button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((_, stacked)) = super::properties::offer(app, ui.ctx()) else { return };
    let fit = app.canvas_rect.is_none_or(|canvas| fits(ui.min_rect().width(), canvas.width()));
    ui.ctx().data_mut(|d| d.insert_temp(shown_id(), fit));
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
