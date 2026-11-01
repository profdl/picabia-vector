//! PICABIA: the Laser window's machine profile editor: the bed, where a job's origin is, the power
//! scale (`S` max, with the connected machine's `$30` one click away), the travel speed, air assist
//! and whether the values were checked on the machine, then the materials' operation settings
//! (speed, power and passes of cut, score and engrave per material, as `laser.profile` keeps them).
//! Shown from `laser.profile`, written with `laser.profile {set}` and saved with the preferences.

use egui::Ui;
use serde_json::{Value, json};

use super::ops::{self, Ops};
use super::status::Status;
use crate::VectorcraftApp;
use crate::i18n;
use crate::picabia_ui::{button, num, readable, wrapped};
use crate::theme::Tokens;
use crate::widgets;

/// A profile number.
fn number(p: &Value, k: &str, default: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(default)
}

/// A number field in the profile; the new value when one was committed.
fn field(ui: &mut Ui, key: &str, name: &str, v: f64, suffix: &str, w: f32) -> Option<f64> {
    widgets::plain_field_named(ui, ("picabia_laser_profile", key), Some(name), v, suffix, 1, w).filter(|v| v.is_finite())
}

/// The profile's rows (`profile` is `laser.profile`'s result, `st` the link's status); the fields
/// to set, when one changed. `material` is the material whose operations the table shows.
pub fn rows(ui: &mut Ui, profile: &Value, st: &Status, material: &mut String) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    let p = profile.get("profile").unwrap_or(&st.profile);
    let mut set = None;
    if let Some(name) = p.get("name").and_then(Value::as_str) {
        crate::picabia_ui::note(ui, name);
    }
    ui.horizontal(|ui| {
        super::lead(ui, tl!("Bed"));
        if let Some(v) = field(ui, "bed_w", tl!("Bed width"), number(p, "bedWidthMm", 584.2), "mm", 84.0) {
            set = Some(json!({"bedWidthMm": v}));
        }
        crate::picabia_ui::note(ui, "×");
        if let Some(v) = field(ui, "bed_h", tl!("Bed height"), number(p, "bedHeightMm", 1219.2), "mm", 84.0) {
            set = Some(json!({"bedHeightMm": v}));
        }
    });
    ui.horizontal(|ui| {
        super::lead(ui, tl!("Origin"));
        let head = p.get("origin").and_then(Value::as_str) != Some("machine");
        if widgets::radio(ui, tl!("Where the head is"), head, true) && !head {
            set = Some(json!({"origin": "head"}));
        }
        if widgets::radio(ui, tl!("Machine zero"), !head, true) && head {
            set = Some(json!({"origin": "machine"}));
        }
    });
    ui.horizontal(|ui| {
        super::lead(ui, tl!("S max"));
        let s_max = number(p, "sMax", 1000.0);
        if let Some(v) = field(ui, "s_max", tl!("S max"), s_max, "", 84.0) {
            set = Some(json!({"sMax": v}));
        }
        if let Some(m) = st.s_max_machine
            && (m - s_max).abs() > 1e-9
            && button(ui, &i18n::fmt(tl!("Use machine value ($30 = {n})"), &[("n", &num(m))]), false, true)
        {
            set = Some(json!({"sMax": m}));
        }
    });
    ui.horizontal(|ui| {
        super::lead(ui, tl!("Travel speed"));
        if let Some(v) = field(ui, "rapid", tl!("Travel speed"), number(p, "rapidMmMin", 3000.0), "mm/min", 120.0) {
            set = Some(json!({"rapidMmMin": v}));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(super::LABEL_W + ui.spacing().item_spacing.x);
        let air = p.get("air").and_then(Value::as_bool).unwrap_or(false);
        if widgets::check(ui, tl!("Air assist (M8/M9)"), air, true) {
            set = Some(json!({"air": !air}));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(super::LABEL_W + ui.spacing().item_spacing.x);
        let checked = p.get("checked").and_then(Value::as_bool).unwrap_or(false);
        if widgets::check(ui, tl!("Checked on the machine"), checked, true) {
            set = Some(json!({"checked": !checked}));
        }
    });
    if p.get("checked").and_then(Value::as_bool) != Some(true) {
        wrapped(ui, tl!("These values are guesses until checked on the machine."), readable(t.warning, &t));
    }
    // The materials' operations: what a job of that material starts from.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        super::lead(ui, tl!("Materials"));
        if let Some(m) = ops::material_dropdown(ui, "profile", material, 140.0) {
            *material = m;
        }
    });
    let mut table = Ops::default();
    if let Some(d) = profile.get("materialDefaults").and_then(|d| d.get(material.as_str())) {
        table.read(d);
    }
    if let Some(saved) = p.get("materials").and_then(|m| m.get(material.as_str())) {
        table.read(saved);
    }
    if ops::table(ui, "profile", &mut table) {
        set = Some(json!({"materials": {material.as_str(): table.to_json()}}));
    }
    set
}

/// Write `set` into the profile; the host saves it with the preferences.
pub fn write(app: &mut VectorcraftApp, set: Value) {
    if app.run("laser.profile", json!({"set": set})).is_ok() {
        app.prefs_changed = true;
    }
}
