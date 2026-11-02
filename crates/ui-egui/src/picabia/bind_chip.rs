//! PICABIA: the binding chip beside every number field of a modifier row (and of the stencil
//! settings): it shows whether the field is bound, and opens a popover to bind it to a parameter
//! or an expression, or to unbind it (`docs/picabia/parameters.md`).
//!
//! The chip is small and unobtrusive: a hollow dot while the field is free, a filled dot when a
//! parameter drives it, a filled square for an expression, in the theme's warning colour when the
//! binding fails (the field then uses its own value; the tooltip says why). It is a button, so Tab
//! reaches it and Enter or Space opens the popover. The widget only reports what the user asked
//! ([`ChipAction`]); the caller runs `modifier.bind` / `modifier.unbind`, so it is one undo step.

use egui::{Rect, Sense, Stroke, Ui, vec2};
use vectorcraft_modifiers::{Binding, BoundField, ParamKind, Parameters, Scope};

use crate::panels::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets;

/// The chip's size, points.
pub const CHIP: f32 = 14.0;

/// What the user asked of a field's binding.
#[derive(Clone, Debug, PartialEq)]
pub enum ChipAction {
    Bind(Binding),
    Unbind,
}

/// One field's binding, as the chip shows it.
pub struct Chip<'a> {
    /// Unique per field on screen.
    pub id: egui::Id,
    /// The field's label (translated), for the tooltip and screen readers.
    pub label: &'a str,
    /// What the row's `bind` says.
    pub binding: Option<&'a Binding>,
    /// What the evaluation made of it.
    pub bound: Option<&'a BoundField>,
    /// The field gives each copy its own value from an expression reading `i` or `n`.
    pub per_copy_field: bool,
    /// The document's parameters (the popover lists them; expressions are checked against them).
    pub params: &'a Parameters,
    pub scope: &'a Scope,
}

/// How a chip looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Free,
    Param,
    Expr,
    Failed,
}

impl Chip<'_> {
    pub fn look(&self) -> Look {
        match (self.binding, self.bound.and_then(|b| b.error.as_ref())) {
            (None, _) => Look::Free,
            (Some(_), Some(_)) | (Some(Binding::Other(_)), _) => Look::Failed,
            (Some(Binding::Param(_)), None) => Look::Param,
            (Some(Binding::Expr(_)), None) => Look::Expr,
        }
    }

    /// The tooltip: what drives the field and what it came to.
    pub fn tip(&self) -> String {
        let what = match self.binding {
            None => return crate::i18n::fmt(tl!("Bind {field} to a parameter or an expression"), &[("field", self.label)]),
            Some(Binding::Param(p)) => crate::i18n::fmt(tl!("{field} = parameter {name}"), &[("field", self.label), ("name", p)]),
            Some(Binding::Expr(e)) => format!("{} = {e}", self.label),
            Some(Binding::Other(_)) => crate::i18n::fmt(tl!("{field}: a binding this version can't use"), &[("field", self.label)]),
        };
        match self.bound.and_then(|b| b.error.as_deref()) {
            Some(e) => format!("{what}\n{e}"),
            None if self.bound.is_some_and(|b| b.per_copy) => format!("{what}\n{}", tl!("A value per copy")),
            None => what,
        }
    }
}

/// The text a bound field shows in place of its editor: `fmt` of the value used, or "per copy".
/// `None` when the field isn't bound.
pub fn shown_value(binding: Option<&Binding>, bound: Option<&BoundField>, own: f64, fmt: &dyn Fn(f64) -> String) -> Option<String> {
    binding?;
    Some(match bound {
        Some(b) if b.per_copy && b.error.is_none() => tl!("per copy").to_string(),
        Some(b) => fmt(b.value.unwrap_or(own)),
        None => fmt(own),
    })
}

/// A bound field's read-only value, `width` wide (accent text; warning text when it failed).
pub fn bound_value(ui: &mut Ui, text: &str, failed: bool, width: f32) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let colour = if failed { t.warning } else { t.accent };
    let (r, resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    ui.painter().rect_stroke(r.shrink(0.5), f32::from(t.radius), Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(11.5), colour);
    let pos = egui::pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0);
    ui.painter().with_clip_rect(r.shrink(2.0)).galley(pos, g, colour);
    resp
}

/// The chip and its popover; the action the user took, if any.
pub fn chip(ui: &mut Ui, c: &Chip) -> Option<ChipAction> {
    let t = Tokens::get(ui.ctx());
    let look = c.look();
    let (r, resp) = ui.allocate_exact_size(vec2(CHIP, CHIP), Sense::click());
    let label = c.tip();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    paint(ui, r, look, resp.hovered() || resp.has_focus(), &t);
    let resp = resp.on_hover_text(&label);
    let mut action = None;
    egui::Popup::from_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_memory(resp.clicked().then_some(egui::SetOpenCommand::Toggle))
        .show(|ui| {
            action = popover(ui, c);
            if action.is_some() {
                ui.close();
            }
        });
    action
}

fn paint(ui: &Ui, r: Rect, look: Look, hot: bool, t: &Tokens) {
    let p = ui.painter();
    let c = r.center();
    let colour = match look {
        Look::Free if hot => t.text,
        Look::Free => t.text_dim,
        Look::Param | Look::Expr => t.accent,
        Look::Failed => t.warning,
    };
    match look {
        Look::Free => {
            p.circle_stroke(c, 3.5, Stroke::new(1.2, colour));
        }
        Look::Param => {
            p.circle_filled(c, 4.5, colour);
        }
        Look::Expr | Look::Failed => {
            p.rect_filled(Rect::from_center_size(c, vec2(9.0, 9.0)), 2.0, colour);
        }
    }
    if hot {
        p.rect_stroke(r, 3.0, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    }
}

/// The popover: a parameter to bind, an expression to type, Unbind; the action taken.
fn popover(ui: &mut Ui, c: &Chip) -> Option<ChipAction> {
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(220.0);
    ui.set_max_width(260.0);
    let mut action = None;
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Bind {field}"), &[("field", c.label)])).strong().size(12.0));
    if let Some(e) = c.bound.and_then(|b| b.error.as_deref()) {
        ui.label(egui::RichText::new(e).size(11.0).color(t.warning));
    }
    // Parameters: the number ones, and text ones that read as numbers.
    widgets::field_label(ui, tl!("Parameter"));
    let names: Vec<&str> = c
        .params
        .list
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::Number { .. }) || c.scope.get(&p.name).is_some_and(|r| r.value.number().is_ok()))
        .map(|p| p.name.as_str())
        .collect();
    if names.is_empty() {
        widgets::dim_label(ui, tl!("No number parameters yet: add one in the Variables panel"));
    } else {
        let current = match c.binding {
            Some(Binding::Param(p)) => p.as_str(),
            _ => "",
        };
        if let Some(name) = widgets::dropdown_names(ui, c.id.with("param"), current, &names, 200.0).and_then(|i| names.get(i))
            && !name.eq_ignore_ascii_case(current)
        {
            action = Some(ChipAction::Bind(Binding::Param((*name).to_string())));
        }
    }
    // An expression, checked as it is committed.
    widgets::field_label(ui, tl!("Expression"));
    let current = match c.binding {
        Some(Binding::Expr(e)) => e.clone(),
        _ => String::new(),
    };
    let err_key = format!("pc-bind-err-{:?}", c.id);
    if let Some(text) = widgets::text_field_named(ui, c.id.with("expr"), Some(tl!("Expression")), Some(&current), 200.0, 1) {
        match vectorcraft_modifiers::expr::Expr::parse_binding(&text, c.scope) {
            Ok(_) => {
                set_pstate::<Option<String>>(ui.ctx(), &err_key, None);
                action = Some(ChipAction::Bind(Binding::Expr(text)));
            }
            Err(e) => set_pstate(ui.ctx(), &err_key, Some(e.to_string())),
        }
    }
    if let Some(e) = pstate::<Option<String>>(ui.ctx(), &err_key) {
        ui.label(egui::RichText::new(e).size(11.0).color(t.warning));
    }
    let hint = if c.per_copy_field {
        tl!("i = copy number, n = copies (a value per copy), t = time (0 for now), value = the field's own value, and parameter names")
    } else {
        tl!("n = copies made above, t = time (0 for now), value = the field's own value, and parameter names")
    };
    ui.label(egui::RichText::new(hint).size(10.5).color(t.text_dim));
    // Ramps come later (`Binding::Other` keeps a newer file's).
    ui.add_enabled(false, egui::Label::new(egui::RichText::new(tl!("Ramp: coming later")).size(10.5)));
    if c.binding.is_some() {
        ui.separator();
        if widgets::flat_button(ui, tl!("Unbind"), 80.0).clicked() {
            action = Some(ChipAction::Unbind);
        }
    }
    action
}
