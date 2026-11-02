//! PICABIA (PIC3.4): the grouped Add Modifier menu, one builder for every place that adds a
//! modifier: the Properties panel's "+ Add Modifier" (`super::properties`), the task bar's
//! Modifiers button (`super::task_bar`), the Appearance panel's Add New Effect ▸ Modifiers
//! (`super::appearance::add_menu`) and Object ▸ Modifiers (`super::menu::object_items`, the same
//! table as menu-tree items). Also the code-drawn glyph the task bar button and the canvas badge
//! show (a stack of copies), so no image asset is added.
//!
//! The groups say what a row does: Repeat makes copies, Shape reshapes the paths (and screens
//! them), Make turns the object into something else (a stencil sheet, a physics body).

use egui::{Color32, Painter, Rect, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// One entry: its label, the kind `modifier.add` adds and the screen preset it fills in.
#[derive(Clone, Copy, Debug)]
pub struct AddItem {
    pub label: &'static str,
    pub kind: &'static str,
    pub preset: Option<&'static str>,
}

const fn kind(label: &'static str, kind: &'static str) -> AddItem {
    AddItem { label, kind, preset: None }
}

const fn preset(label: &'static str, preset: &'static str) -> AddItem {
    AddItem { label, kind: "screen", preset: Some(preset) }
}

/// The groups, in menu order, with their entries. Make also offers Physics ▸ Add Physics in the
/// panels' menus (Object ▸ Physics has it in the menu bar).
pub const GROUPS: &[(&str, &[AddItem])] = &[
    (
        "Repeat",
        &[
            kind("Linear Array", "linearArray"),
            kind("Circular Array", "circularArray"),
            kind("Grid Array", "gridArray"),
            kind("Mirror", "mirror"),
            kind("Wallpaper", "wallpaper"),
            kind("Copy to Shape", "copyToShape"),
            kind("Transform Each", "transformEach"),
            kind("Randomize", "randomize"),
        ],
    ),
    (
        "Shape",
        &[
            kind("Resample", "resample"),
            kind("Trim", "trim"),
            kind("Tidy", "tidy"),
            kind("Deform", "deform"),
            kind("Displace", "displace"),
            kind("Screen", "screen"),
            preset("Ben-Day Dots", "benDay"),
            preset("Line Fill", "lineFill"),
        ],
    ),
    ("Make", &[kind("Stencil", "stencil")]),
];

/// `modifier.add`'s params for `item` (the selection's objects).
pub fn params(item: &AddItem) -> serde_json::Value {
    match item.preset {
        Some(p) => json!({"kind": item.kind, "preset": p}),
        None => json!({"kind": item.kind}),
    }
}

/// The grouped entries, inside a popup or submenu: each group's name, dim, then its entries; a
/// click adds that modifier to the selected objects (`modifier.add`, one undo step) and closes the
/// menu. Make ends with Physics ▸ Add Physics.
pub fn items(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    for (i, (group, entries)) in GROUPS.iter().enumerate() {
        if i > 0 {
            ui.separator();
        }
        ui.label(egui::RichText::new(tl!(group)).size(11.0).color(t.text_dim));
        for item in *entries {
            if ui.button(tl!(item.label)).clicked() {
                if let Err(e) = app.run("modifier.add", params(item)) {
                    app.ui.status = e;
                }
                ui.close();
            }
        }
    }
    ui.menu_button(tl!("Physics"), |ui| {
        if ui.button(tl!("Add Physics")).clicked() {
            if let Err(e) = app.run("physics.set", json!({"props": {"enabled": true}})) {
                app.ui.status = e;
            }
            ui.close();
        }
    });
}

/// The modifiers glyph in `rect`: three squares stepping down to the right, the front one filled
/// with `bg` so it hides the others' corners (a stack of copies).
pub fn paint_glyph(painter: &Painter, rect: Rect, color: Color32, bg: Color32) {
    let side = rect.width().min(rect.height()) * 0.56;
    let step = (rect.width().min(rect.height()) - side) / 2.0;
    let stroke = Stroke::new(1.3, color);
    for k in 0..3 {
        let min = pos2(rect.left() + step * k as f32, rect.top() + step * k as f32);
        let r = Rect::from_min_size(min, vec2(side, side));
        painter.rect_filled(r, 1.5, bg);
        painter.rect_stroke(r, 1.5, stroke, egui::StrokeKind::Inside);
    }
}
