//! PICABIA: the Parameters section of VectorCraft's Variables panel (`panels::variables::show`
//! calls [`section`] between its Variables and Data Sets lists).
//!
//! One row per document parameter: its name (editable: a rename carries the bindings that read
//! it), a slider and a field for a number (the slider drags live and commits one undo step on
//! release) or a field for a text, the VectorCraft variable it takes its value from while a data
//! set is active, what that gave, and a bin. "+ Number" and "+ Text" add one. Every edit is a
//! `param.*` command, so an agent does the same (`docs/picabia/parameters.md`).

use egui::{Ui, vec2};
use serde_json::json;
use vectorcraft_modifiers::{ParamKind, Parameters, Scope};

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, Live};

/// The name field's width.
const NAME_W: f32 = 84.0;
/// The value field's width.
const VALUE_W: f32 = 56.0;

/// What the section shows, read from the document once per frame.
struct Snapshot {
    params: Parameters,
    scope: Scope,
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

/// The Parameters section.
pub fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(snap) = snapshot(app) else { return };
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Parameters"));
    if snap.params.list.is_empty() {
        widgets::dim_label(ui, tl!("None: add one, then bind a modifier field to it with the chip beside the field"));
    }
    for p in &snap.params.list {
        let name = p.name.clone();
        let resolved = snap.scope.get(&name);
        let from_data = resolved.is_some_and(|r| r.from_dataset);
        ui.horizontal(|ui| {
            if let Some(new) = widgets::text_field_named(ui, ("pc-param-name", &name), Some(tl!("Parameter name")), Some(&name), NAME_W, 1)
                && !new.is_empty()
            {
                app.run("param.set", json!({"name": name, "rename": new})).ok();
            }
            match &p.kind {
                ParamKind::Number { value, min, max, step } => {
                    let shown = match resolved.map(|r| r.value.number()) {
                        Some(Ok(v)) => v,
                        _ => *value,
                    };
                    let mut v = shown;
                    let slider_w = (ui.available_width() - VALUE_W - 30.0).max(40.0);
                    ui.spacing_mut().slider_width = slider_w;
                    let mut slider = egui::Slider::new(&mut v, *min..=*max).show_value(false);
                    if *step > 0.0 {
                        slider = slider.step_by(*step);
                    }
                    // A value from the data set is the data set's: change it there.
                    let resp = ui.add_enabled_ui(!from_data, |ui| widgets::slider(ui, slider)).inner;
                    let phase = if resp.drag_stopped() {
                        Live::Released
                    } else if resp.dragged() {
                        Live::Dragging
                    } else if resp.changed() {
                        Live::Released
                    } else {
                        Live::Idle
                    };
                    if phase != Live::Idle && v.is_finite() {
                        crate::panels::live_run(app, "Set Parameter", "param.set", json!({"name": name, "value": v.clamp(*min, *max)}), phase);
                    }
                    let typed = ui
                        .add_enabled_ui(!from_data, |ui| {
                            widgets::plain_field_named(ui, ("pc-param-value", &name), Some(&name), shown, "", decimals(*step), VALUE_W)
                        })
                        .inner;
                    if let Some(n) = typed.filter(|n| n.is_finite()) {
                        app.run("param.set", json!({"name": name, "value": n.clamp(*min, *max)})).ok();
                    }
                }
                ParamKind::Text { value } => {
                    let shown = match resolved.map(|r| &r.value) {
                        Some(vectorcraft_modifiers::ParamValue::Text(s)) => s.clone(),
                        _ => value.clone(),
                    };
                    let w = (ui.available_width() - 30.0).max(60.0);
                    let typed = ui
                        .add_enabled_ui(!from_data, |ui| widgets::text_field_named(ui, ("pc-param-text", &name), Some(&name), Some(&shown), w, 1))
                        .inner;
                    if let Some(s) = typed {
                        app.run("param.set", json!({"name": name, "value": s})).ok();
                    }
                }
            }
            if widgets::icon_button(ui, "trash-2", tl!("Remove Parameter"), false, 22.0).clicked() {
                app.run("param.remove", json!({"name": name})).ok();
            }
        });
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
                app.run("param.set", json!({"name": name, "source": source})).ok();
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
    ui.horizontal(|ui| {
        let full = snap.params.list.len() >= vectorcraft_modifiers::params::MAX_PARAMS;
        if ui.add_enabled(!full, egui::Button::new(tl!("+ Number")).min_size(vec2(70.0, 22.0))).clicked() {
            app.run("param.add", json!({"name": fresh_name(&snap.params), "type": "number", "value": 0})).ok();
        }
        if ui.add_enabled(!full, egui::Button::new(tl!("+ Text")).min_size(vec2(70.0, 22.0))).clicked() {
            app.run("param.add", json!({"name": fresh_name(&snap.params), "type": "text"})).ok();
        }
    });
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
