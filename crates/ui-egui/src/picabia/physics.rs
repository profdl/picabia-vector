//! PICABIA: the selection's physics settings in the Appearance panel, under the modifier rows.
//!
//! One row when a selected object has physics settings ("Physics: Dynamic"; "Mixed" when the
//! selected objects' body types differ): the eye turns them on and off, the disclosure opens the
//! fields (tlprocedural's Physics settings, values clamped to its ranges), and the row has
//! Simulate and Clear. Every edit is a `physics.*` command for every selected object, so it is one
//! undo step and agents can do the same. Objects without settings show no row: Object ▸ Physics ▸
//! Add Physics (or Settings…) gives them some.

use egui::{Rect, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

const ROW: f32 = 30.0;
const EYE_W: f32 = 26.0;

const BODIES: &[(&str, &str)] = &[("dynamic", "Dynamic"), ("static", "Static"), ("kinematic", "Kinematic")];
const COLLIDERS: &[(&str, &str)] = &[("auto", "Auto"), ("precise", "Precise"), ("fast", "Fast"), ("box", "Box")];

/// How a field is edited.
#[derive(Clone, Copy)]
enum F {
    /// A number within `min..=max` with `decimals`, and a suffix.
    Num(f64, f64, usize, &'static str),
    Bool,
    Choice(&'static [(&'static str, &'static str)]),
}

/// The fields: `(key, label, how)`, with tlprocedural's ranges.
const FIELDS: &[(&str, &str, F)] = &[
    ("bodyType", "Body", F::Choice(BODIES)),
    ("colliderType", "Collider", F::Choice(COLLIDERS)),
    ("gravityScale", "Gravity", F::Num(0.0, 2.0, 2, "×")),
    ("restitution", "Bounce", F::Num(0.0, 1.0, 2, "")),
    ("friction", "Friction", F::Num(0.0, 1.0, 2, "")),
    ("mass", "Extra mass", F::Num(0.0, 10_000.0, 2, "kg")),
    ("linearDamping", "Drag", F::Num(0.0, 20.0, 2, "")),
    ("angularDamping", "Spin drag", F::Num(0.0, 20.0, 2, "")),
    ("sensor", "Sensor (passes through)", F::Bool),
];

/// The selected objects (layers aside) and the first one's settings, when any selected object has
/// physics settings; `mixed`: their body types differ.
struct View {
    ids: Vec<NodeId>,
    first: Value,
    mixed: bool,
    damaged: Option<String>,
}

fn view(app: &VectorcraftApp) -> Option<View> {
    let st = app.session.active()?;
    let ids: Vec<NodeId> = st.selection.subjects().iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())).collect();
    let mut first: Option<Value> = None;
    let mut kinds = std::collections::BTreeSet::new();
    let mut damaged = None;
    for id in &ids {
        match st.doc.node(*id).and_then(vectorcraft_physics::node_settings) {
            None => {
                kinds.insert("none".to_string());
            }
            Some(Ok((s, _))) => {
                kinds.insert(s.body_type.as_str().to_string());
                if first.is_none() {
                    first = Some(s.to_value());
                }
            }
            Some(Err(e)) => damaged = Some(e),
        }
    }
    if first.is_none() && damaged.is_none() {
        return None;
    }
    Some(View { ids, first: first.unwrap_or(Value::Null), mixed: kinds.len() > 1, damaged })
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

/// The physics row (and its fields when open), or nothing when no selected object has settings.
pub fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(v) = view(app) else { return };
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([pos2(r.left() + EYE_W, r.top()), pos2(r.left() + EYE_W, r.bottom())], egui::Stroke::new(1.0, t.border));
    let enabled = v.first.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let er = Rect::from_center_size(pos2(r.left() + EYE_W / 2.0, r.center().y), vec2(16.0, 16.0));
    if icon_hit(ui, er, "pp-eye", if enabled { "eye" } else { "eye-off" }, tl!("Click to turn physics on or off")).clicked() {
        app.run("physics.set", json!({"ids": ids_json(&v.ids), "props": {"enabled": !enabled}})).ok();
    }
    let open = app.picabia.physics_open;
    let cr = Rect::from_center_size(pos2(r.left() + EYE_W + 12.0, r.center().y), vec2(12.0, 12.0));
    if icon_hit(ui, cr, "pp-chev", if open { "chevron-down" } else { "chevron-right" }, tl!("Show the settings")).clicked() || resp.double_clicked() {
        app.picabia.physics_open = !open;
    }
    let body = v.first.get("bodyType").and_then(Value::as_str).unwrap_or("dynamic");
    let shown = if v.mixed { tl!("Mixed") } else { BODIES.iter().find(|(k, _)| *k == body).map_or(body, |(_, l)| tl!(l)) };
    let label = format!("{} {shown}", tl!("Physics:"));
    let col = if enabled { t.text_strong } else { t.text_dim };
    ui.painter().text(pos2(r.left() + EYE_W + 24.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), col);
    let br = Rect::from_min_size(pos2(r.right() - 84.0, r.center().y - 11.0), vec2(80.0, 22.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if widgets::flat_button(&mut child, tl!("Simulate"), 76.0).on_hover_text(tl!("Drop and settle: run the physics until everything rests")).clicked()
    {
        simulate(app);
    }
    if let Some(e) = &v.damaged {
        let g =
            ui.painter().layout(format!("{} {e}", tl!("Physics:")), egui::FontId::proportional(11.5), t.warning, ui.available_width() - EYE_W - 30.0);
        let (nr, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 8.0), Sense::hover());
        ui.painter().galley(pos2(nr.left() + EYE_W + 24.0, nr.center().y - g.size().y / 2.0), g, t.warning);
    }
    if open && !v.first.is_null() {
        fields(app, ui, &v);
    }
}

/// Run Simulate and say what happened in the status line.
fn simulate(app: &mut VectorcraftApp) {
    match app.run("physics.simulate", json!({})) {
        Ok(out) => {
            let moved = out.get("moved").and_then(Value::as_array).map_or(0, Vec::len);
            let secs = out.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
            let settled = out.get("settled").and_then(Value::as_bool).unwrap_or(false);
            let n = crate::i18n::trn(crate::i18n::current(), moved as u64, "{n} object moved", "{n} objects moved");
            let state = if settled { tl!("settled") } else { tl!("still moving at the step limit") };
            app.ui.status = format!("{n} ({secs:.1} s, {state})");
        }
        Err(e) => app.ui.status = e,
    }
}

fn fields(app: &mut VectorcraftApp, ui: &mut Ui, v: &View) {
    let t = Tokens::get(ui.ctx());
    let mut change: Option<(String, Value)> = None;
    let mut clear = false;
    egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin { left: (EYE_W + 12.0) as i8, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for (key, label, how) in FIELDS {
            let Some(cur) = v.first.get(*key) else { continue };
            ui.horizontal(|ui| {
                if !matches!(how, F::Bool) {
                    ui.add_sized(vec2(84.0, 22.0), egui::Label::new(egui::RichText::new(tl!(label)).size(11.5).color(t.text)).truncate());
                }
                let fid = ("pp-field", *key);
                let new = match *how {
                    F::Num(lo, hi, decimals, suffix) => widgets::plain_field(ui, fid, cur.as_f64().unwrap_or(0.0), suffix, decimals, 70.0)
                        .filter(|x| x.is_finite())
                        .map(|x| json!(x.clamp(lo, hi))),
                    F::Bool => {
                        let b = cur.as_bool().unwrap_or(false);
                        widgets::check_name(ui, tl!(label), b).then(|| json!(!b))
                    }
                    F::Choice(options) => {
                        let c = cur.as_str().unwrap_or_default();
                        let shown = options.iter().find(|(value, _)| *value == c).map_or(c, |(_, l)| tl!(l));
                        let labels: Vec<&str> = options.iter().map(|(_, l)| tl!(l)).collect();
                        widgets::dropdown_names(ui, fid, shown, &labels, 120.0)
                            .and_then(|k| options.get(k))
                            .filter(|(value, _)| *value != c)
                            .map(|(value, _)| json!(value))
                    }
                };
                if let Some(new) = new {
                    change = Some((key.to_string(), new));
                }
            });
        }
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, tl!("Clear Physics"), 100.0).clicked() {
                clear = true;
            }
        });
    });
    if let Some((key, value)) = change {
        app.run("physics.set", json!({"ids": ids_json(&v.ids), "props": {key: value}})).ok();
    }
    if clear {
        app.run("physics.clear", json!({"ids": ids_json(&v.ids)})).ok();
    }
}

fn icon_hit(ui: &mut Ui, r: Rect, id: &str, icon: &str, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    crate::icons::paint(ui, icon, r, if resp.hovered() { t.text_strong } else { t.icon });
    resp.on_hover_text(tip)
}
