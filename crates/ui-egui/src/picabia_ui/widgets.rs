//! PICABIA: the small widgets Picabia's laser, stencil, camera and modifier UI is written with, on the
//! craft framework's components (buttons, checks, radios, selects, number and text fields, menu rows),
//! so the framework's focus, keyboard, pointer routing, accessibility and theme apply to them. Each
//! keeps the call shape the Picabia code used before VectorCraft moved onto the framework: the value
//! in, the committed change (or the click) out.

use craft::behavior::{IconId, Mixed};
use craft::ui::buttons::{Button, ButtonOutput, IconButton};
use craft::ui::choice::{Checkbox, Radio, RadioGroup, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::menu::MenuItem;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::style::{TypeRole, Variant};
use craft::ui::text::{NumberField, TextField};
use egui::{Rect, Response, Ui};
use vectorcraft_doc::Unit;

/// A button's click and its response (for a tooltip or a menu anchored to it).
pub(crate) struct Pressed {
    pub response: Response,
    clicked: bool,
}

impl std::ops::Deref for Pressed {
    type Target = Response;
    fn deref(&self) -> &Response {
        &self.response
    }
}

impl Pressed {
    fn of(out: ButtonOutput) -> Self {
        let clicked = out.clicked();
        Self { response: out.response, clicked }
    }

    /// The button was activated (a click, Enter or Space) this pass.
    pub fn clicked(&self) -> bool {
        self.clicked
    }

    /// The same, with a tool tip under the pointer.
    pub fn on_hover_text(self, tip: &str) -> Self {
        Self { response: self.response.on_hover_text(tip), clicked: self.clicked }
    }
}

/// A quiet button labelled `text` (already translated).
pub(crate) fn flat_button(ui: &mut Ui, text: &str, _width: f32) -> Pressed {
    Pressed::of(Button::new(text.to_string()).variant(Variant::SECONDARY).show(ui))
}

/// The accent button, greyed and not clickable unless `enabled`.
pub(crate) fn primary_button_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Pressed {
    Pressed::of(Button::new(tl!(text).to_string()).variant(Variant::PRIMARY).enabled(enabled).show(ui))
}

/// The quiet secondary button.
pub(crate) fn secondary_button(ui: &mut Ui, text: &str) -> Pressed {
    Pressed::of(Button::new(tl!(text).to_string()).variant(Variant::SECONDARY).enabled(ui.is_enabled()).show(ui))
}

/// The icon button `icon` at `rect` (a row's eye or disclosure), named and tipped `tip` (already
/// translated).
pub(crate) fn icon_at(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: &'static str, tip: &str) -> Pressed {
    Pressed::of(IconButton::new(IconId::new(icon)).label(tip.to_string()).tooltip(tip.to_string()).id_salt(id).show_at(ui, rect))
}

/// An icon button named and tipped `tip` (English, translated here); `selected` draws it pressed.
pub(crate) fn icon_button(ui: &mut Ui, icon: &'static str, tip: &str, selected: bool, _size: f32) -> Pressed {
    let tip = tl!(tip).to_string();
    Pressed::of(IconButton::new(IconId::new(icon)).label(tip.clone()).tooltip(tip).selected(selected).show(ui))
}

/// A check box labelled `label` (English, translated here); whether it was toggled.
pub(crate) fn check(ui: &mut Ui, label: &str, value: bool, enabled: bool) -> bool {
    check_named(ui, tl!(label), value, enabled)
}

/// A check box labelled `name` as it is (a name of the document's, or already translated).
pub(crate) fn check_name(ui: &mut Ui, name: &str, value: bool) -> bool {
    check_named(ui, name, value, true)
}

/// A check box with no text beside it (the row's label is drawn in another column), which screen
/// readers call `name` (already translated); whether it was toggled.
pub(crate) fn check_box(ui: &mut Ui, name: &str, value: bool, enabled: bool) -> bool {
    Checkbox::new(Bind::given(value)).label(name.to_string()).enabled(enabled).show(ui).events.committed().is_some_and(|on| *on != value)
}

fn check_named(ui: &mut Ui, text: &str, value: bool, enabled: bool) -> bool {
    Checkbox::new(Bind::given(value)).label(text.to_string()).enabled(enabled).show(ui).events.committed().is_some_and(|on| *on != value)
}

/// A radio button labelled `label` (English, translated here); whether it was picked.
pub(crate) fn radio(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    let text = tl!(label).to_string();
    let group = RadioGroup::new(text.clone(), Bind::given(selected)).show(ui, |ui, _| {
        Radio::new(true).label(text.clone()).enabled(enabled).show(ui);
    });
    group.output.value().is_some_and(|on| *on && !selected)
}

/// A menu row labelled `label` (English, translated here), checked when `checked`; whether it ran.
pub(crate) fn menu_item(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let mut row = MenuItem::new(tl!(label).to_string()).enabled(enabled);
    if checked {
        row = row.check(true);
    }
    row.show(ui).ran
}

/// A select of `options` (shown as they are), showing `current`; the index picked.
pub(crate) fn dropdown_names(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_names_named(ui, id, None, current, options, width)
}

/// [`dropdown_names`] that screen readers call `name`.
pub(crate) fn dropdown_names_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: Option<&str>,
    current: &str,
    options: &[&str],
    _width: f32,
) -> Option<usize> {
    let entries: Vec<Entry<usize>> = options.iter().enumerate().map(|(i, o)| Entry::new(i, o.to_string())).collect();
    let at = options.iter().position(|o| *o == current);
    let picked = Select::new(name.unwrap_or(current).to_string(), Bind::given(at.map_or(Mixed::Mixed, Mixed::Same)), &entries)
        .id_salt(id)
        .show(ui)
        .events
        .committed()
        .copied();
    picked.filter(|p| Some(*p) != at)
}

/// [`dropdown_names_named`] of interface labels (English, translated here).
pub(crate) fn dropdown_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: Option<&str>,
    current: &str,
    options: &[&str],
    width: f32,
) -> Option<usize> {
    let shown: Vec<&str> = options.iter().map(|o| tl!(o)).collect();
    dropdown_names_named(ui, id, name, tl!(current), &shown, width)
}

/// A number field showing `value` (points) in `unit`; the new value (points) when one is committed.
pub(crate) fn num_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, _width: f32) -> Option<f64> {
    let lengths = LengthUnits::new(UnitKind::Points, crate::canvas::rulers::unit_kind(unit), 72.0);
    NumberField::length(String::new(), Bind::given(value), lengths).id_salt(id).show(ui).edit.events.committed().copied().flatten()
}

/// A number field with `decimals` decimals and `suffix` after the number; the new value when one is
/// committed.
pub(crate) fn plain_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f64,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    plain_field_named(ui, id, None, value, suffix, decimals, width)
}

/// [`plain_field`] that screen readers call `name`.
pub(crate) fn plain_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: Option<&str>,
    value: f64,
    suffix: &str,
    decimals: usize,
    _width: f32,
) -> Option<f64> {
    let mut field = NumberField::new(name.unwrap_or_default().to_string(), Bind::given(Some(value))).max_decimals(decimals).id_salt(id);
    if !suffix.is_empty() {
        field = field.unit(suffix.to_string());
    }
    let new = field.show(ui).edit.events.committed().copied().flatten();
    new.filter(|n| n.is_finite() && *n != value)
}

/// A text field showing `value` (blank when `None`), `rows` lines tall; the new text, trimmed, when a
/// change is committed.
pub(crate) fn text_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: Option<&str>,
    value: Option<&str>,
    _width: f32,
    rows: usize,
) -> Option<String> {
    let shown = value.unwrap_or_default();
    let mut field = TextField::new(name.unwrap_or_default().to_string(), Bind::given(shown.to_string())).id_salt(id);
    if rows > 1 {
        field = field.lines(rows);
    }
    let new = field.show(ui).events.committed().cloned();
    new.map(|t| t.trim().to_string()).filter(|t| t != shown)
}

/// A one-line text field that keeps the text exactly as typed; the new text when a change is
/// committed.
pub(crate) fn exact_text_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: Option<&str>,
    value: &str,
    _width: f32,
) -> Option<String> {
    let new = TextField::new(name.unwrap_or_default().to_string(), Bind::given(value.to_string())).id_salt(id).show(ui).events.committed().cloned();
    new.filter(|t| t != value)
}

/// A hex colour field (`RRGGBB`); the new text when a change is committed.
pub(crate) fn hex_field_named(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, name: Option<&str>, hex: &str) -> Option<String> {
    exact_text_field_named(ui, id, name, hex, 70.0)
}

/// A row of tabs (`tabs` translated already), `current` picked; the tab picked.
pub(crate) fn tab_bar(ui: &mut Ui, tabs: &[&str], current: usize) -> Option<usize> {
    let entries: Vec<Entry<usize>> = tabs.iter().enumerate().map(|(i, t)| Entry::new(i, t.to_string())).collect();
    let name = tabs.get(current).copied().unwrap_or_default().to_string();
    let picked = Segmented::new(name, Bind::given(current), &entries).show(ui).events.committed().copied();
    picked.filter(|p| *p != current)
}

/// A section header that opens and closes (English `text`, translated here); whether it was clicked.
pub(crate) fn section_toggle(ui: &mut Ui, text: &str, open: bool) -> bool {
    let icon = if open { "chevron-down" } else { "chevron-right" };
    Button::new(tl!(text).to_string()).icon(IconSpec::svg(IconId::new(icon))).variant(Variant::GHOST).show(ui).clicked()
}

/// A divider across the row.
pub(crate) fn divider(ui: &mut Ui) {
    Divider::horizontal().show(ui);
}

/// A dim line of text (already translated).
pub(crate) fn dim_label(ui: &mut Ui, text: &str) -> Response {
    Text::new(text.to_string()).shade(Shade::Muted).show(ui)
}

/// A field's caption (already translated).
pub(crate) fn field_label(ui: &mut Ui, text: &str) -> Response {
    Text::new(text.to_string()).show(ui)
}

/// A subheading (already translated).
pub(crate) fn subheader(ui: &mut Ui, text: &str) {
    Text::new(text.to_string()).role(TypeRole::Heading).show(ui);
}
