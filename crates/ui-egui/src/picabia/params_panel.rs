//! PICABIA: the Parameters section of VectorCraft's Variables panel (`panels::variables::show`
//! shows a [`Section`] at the top of its body, above the variable table that fills the panel).
//!
//! One row per document parameter: its name (editable: a rename carries the bindings that read
//! it), a slider and a field for a number (the slider drags live and commits one undo step on
//! release) or a field for a text, the VectorCraft variable it takes its value from while a data
//! set is active, what that gave, and a bin. "+ Number" and "+ Text" add one. Every edit is a
//! `param.*` command, so an agent does the same (`docs/picabia/parameters.md`).

use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::value_drag::Slider;
use egui::Ui;
use serde_json::json;
use vectorcraft_modifiers::{ParamKind, Parameters, Scope};

use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};
use crate::picabia_ui::Tokens;
use crate::picabia_ui::widgets;

/// The name field's width.
const NAME_W: f32 = 84.0;
/// The value field's width.
const VALUE_W: f32 = 56.0;

/// What the section shows, read from the document once per frame.
struct Snapshot {
    params: Parameters,
    scope: std::sync::Arc<Scope>,
    variables: Vec<String>,
}

fn snapshot(app: &VectorcraftApp) -> Option<Snapshot> {
    let st = app.session.active()?;
    Some(Snapshot {
        params: Parameters::of_doc(&st.doc),
        scope: Scope::of_doc(&st.doc),
        variables: st.doc.variables.variables.iter().map(|v| v.name.clone()).collect(),
    })
}

/// Run `cmd`; a refusal (a rename to a taken name, a value out of range) goes to the status bar.
fn run(app: &mut VectorcraftApp, cmd: &str, params: serde_json::Value) {
    match app.run(cmd, params) {
        Ok(v) => {
            if let Some(w) = v.get("warning").and_then(serde_json::Value::as_str) {
                app.ui.status = w.to_string();
            }
        }
        Err(e) => app.ui.status = e.to_string(),
    }
}

/// A name `param.add` will take: `param1`, `param2`… (the first free one).
fn fresh_name(params: &Parameters) -> String {
    (1..=vectorcraft_modifiers::params::MAX_PARAMS + 1)
        .map(|k| format!("param{k}"))
        .find(|n| params.get(n).is_none())
        .unwrap_or_else(|| "param".into())
}

/// Decimals to show for a step of `step` (0: two).
fn decimals(step: f64) -> usize {
    if step <= 0.0 || !step.is_finite() {
        return 2;
    }
    let mut d = 0;
    let mut s = step;
    while d < 4 && (s - s.round()).abs() > 1e-9 {
        s *= 10.0;
        d += 1;
    }
    d
}

/// What the section does once it is drawn: its commands, and its sliders' edits.
enum Act {
    Run(&'static str, serde_json::Value),
    Slide { name: String, edit: Box<craft::ui::contract::EditResponse<f64>>, min: f64, max: f64 },
}

/// The Parameters section: read from the document before the panel draws (the panel's body can't
/// borrow the app), drawn inside it ([`Section::show`]), its edits run after ([`Section::apply`]).
pub struct Section {
    snap: Option<Snapshot>,
    acts: Vec<Act>,
}

impl Section {
    /// The section of the active document.
    pub fn of(app: &VectorcraftApp) -> Self {
        Section { snap: snapshot(app), acts: Vec::new() }
    }

    /// Run what the section's rows asked for.
    pub fn apply(self, app: &mut VectorcraftApp) {
        for act in self.acts {
            match act {
                Act::Run(cmd, params) => run(app, cmd, params),
                Act::Slide { name, edit, min, max } => {
                    // The slider drags live and commits one undo step on release.
                    apply_edits(app, &*edit, EditPath::Preview, |app, v, phase| {
                        if v.is_finite() {
                            crate::panels::live_run(app, "Set Parameter", "param.set", json!({"name": name, "value": v.clamp(min, max)}), phase);
                        }
                    });
                }
            }
        }
    }

    /// The rows, in a container of the framework's, so the panel's stack measures them and lays its
    /// lists out around them.
    pub fn show(&mut self, ui: &mut Ui) {
        if self.snap.is_some() {
            craft::ui::layout_components::Stack::new().fill_width().show(ui, |ui, _| self.rows(ui));
        }
    }

    fn rows(&mut self, ui: &mut Ui) {
        let Some(snap) = &self.snap else { return };
        let acts = &mut self.acts;
        let t = Tokens::get(ui.ctx());
        widgets::subheader(ui, tl!("Parameters"));
        if snap.params.list.is_empty() {
            widgets::dim_label(ui, tl!("None: add one, then bind a modifier field to it with the chip beside the field"));
        }
        for p in &snap.params.list {
            let name = p.name.clone();
            let resolved = snap.scope.get(&name);
            let from_data = resolved.is_some_and(|r| r.from_dataset);
            // A number's slider goes on a line of its own under the row: (shown, min, max, step).
            let mut slider_line = None;
            ui.horizontal(|ui| {
                if let Some(new) = widgets::text_field_named(ui, ("pc-param-name", &name), Some(tl!("Parameter name")), Some(&name), NAME_W, 1)
                    && !new.is_empty()
                {
                    acts.push(Act::Run("param.set", json!({"name": name, "rename": new})));
                }
                match &p.kind {
                    ParamKind::Number { value, min, max, step } => {
                        let shown = match resolved.map(|r| r.value.number()) {
                            Some(Ok(v)) => v,
                            _ => *value,
                        };
                        slider_line = Some((shown, *min, *max, *step));
                        let typed = ui
                            .add_enabled_ui(!from_data, |ui| {
                                widgets::plain_field_named(ui, ("pc-param-value", &name), Some(&name), shown, "", decimals(*step), VALUE_W)
                            })
                            .inner;
                        if let Some(n) = typed.filter(|n| n.is_finite()) {
                            acts.push(Act::Run("param.set", json!({"name": name, "value": n.clamp(*min, *max)})));
                        }
                    }
                    ParamKind::Text { value } => {
                        let shown = match resolved.map(|r| &r.value) {
                            Some(vectorcraft_modifiers::ParamValue::Text(s)) => s.clone(),
                            _ => value.clone(),
                        };
                        let w = (ui.available_width() - 30.0).max(60.0);
                        let typed = ui
                            .add_enabled_ui(!from_data, |ui| widgets::exact_text_field_named(ui, ("pc-param-text", &name), Some(&name), &shown, w))
                            .inner;
                        if let Some(s) = typed {
                            acts.push(Act::Run("param.set", json!({"name": name, "value": s})));
                        }
                    }
                }
                if widgets::icon_button(ui, "trash-2", "Remove Parameter", false, 22.0).clicked() {
                    acts.push(Act::Run("param.remove", json!({"name": name})));
                }
            });
            if let Some((shown, min, max, step)) = slider_line {
                let mut slider =
                    Slider::new(Bind::given(shown), min..=max).label(format!("{name} {}", tl!("slider"))).id_salt(("pc-param-slider", &name));
                if step > 0.0 {
                    slider = slider.step(step);
                }
                // A value from the data set is the data set's: change it there.
                let edit = slider.enabled(!from_data).show(ui);
                acts.push(Act::Slide { name: name.clone(), edit: Box::new(edit), min, max });
            }
            // Its data source: a VectorCraft variable whose value in the active data set it takes.
            ui.horizontal(|ui| {
                ui.add_space(NAME_W + ui.spacing().item_spacing.x);
                let none = tl!("No data source");
                let mut labels: Vec<&str> = vec![none];
                labels.extend(snap.variables.iter().map(String::as_str));
                let current = p.source.as_deref().unwrap_or(none);
                if let Some(k) = widgets::dropdown_names(ui, ("pc-param-source", &name), current, &labels, 120.0)
                    && labels.get(k).is_some_and(|l| *l != current)
                {
                    let source = if k == 0 { serde_json::Value::Null } else { json!(labels.get(k)) };
                    acts.push(Act::Run("param.set", json!({"name": name, "source": source})));
                }
                if from_data {
                    ui.label(egui::RichText::new(tl!("from the data set")).size(10.5).color(t.text_dim));
                }
            });
            if let Some(w) = resolved.and_then(|r| r.warning.as_deref()) {
                ui.label(egui::RichText::new(w).size(10.5).color(t.warning));
            }
            ui.add_space(2.0);
        }
        Row::new().show(ui, |ui, _| {
            let full = snap.params.list.len() >= vectorcraft_modifiers::params::MAX_PARAMS;
            if crate::picabia_ui::button(ui, tl!("+ Number"), false, !full) {
                acts.push(Act::Run("param.add", json!({"name": fresh_name(&snap.params), "type": "number", "value": 0})));
            }
            if crate::picabia_ui::button(ui, tl!("+ Text"), false, !full) {
                acts.push(Act::Run("param.add", json!({"name": fresh_name(&snap.params), "type": "text"})));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_follow_the_step_and_names_are_fresh() {
        assert_eq!((decimals(1.0), decimals(0.5), decimals(0.25), decimals(0.0), decimals(f64::NAN), decimals(0.0001)), (0, 1, 2, 2, 2, 4));
        let mut ps = Parameters::default();
        assert_eq!(fresh_name(&ps), "param1");
        ps.list.push(vectorcraft_modifiers::Parameter::new("param1", &serde_json::Map::new()).unwrap());
        assert_eq!(fresh_name(&ps), "param2");
    }
}
