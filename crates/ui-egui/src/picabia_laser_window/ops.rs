//! PICABIA: the laser's operation table (cut, score, engrave: on, speed, power, passes) and its
//! materials, shared by the Laser window and the laser export dialog. The values are the commands'
//! (`cut`, `score`, `engrave` params of `laser.gcode`, `laser.start`, `modifier.stencilGcode`,
//! `modifier.stencilTestCard`): the table shows what the command reported until the person edits
//! it, then sends all three.

use egui::Ui;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::widgets;

/// Laser materials: (value written in `material`, label).
pub const MATERIALS: &[(&str, &str)] = &[("paper", "Paper"), ("film", "Film"), ("card", "Card"), ("custom", "Custom")];

/// The label of material `id`, translated (the id itself when unknown).
pub fn material_label(id: &str) -> &str {
    match id {
        "paper" => tl!("Paper"),
        "film" => tl!("Film"),
        "card" => tl!("Card"),
        "custom" => tl!("Custom"),
        other => other,
    }
}

/// One operation's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Op {
    pub enabled: bool,
    /// mm/min, 1-20000.
    pub speed: f64,
    /// %, 0-100.
    pub power: f64,
    /// 1-20.
    pub passes: u32,
}

impl Op {
    const fn new(speed: f64, power: f64, passes: u32) -> Self {
        Op { enabled: true, speed, power, passes }
    }

    /// Take what `v` (`{enabled, speedMmMin, powerPct, passes}`) sets validly.
    fn read(&mut self, v: &Value) {
        let num = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(b) = v.get("enabled").and_then(Value::as_bool) {
            self.enabled = b;
        }
        if let Some(s) = num("speedMmMin") {
            self.speed = s.clamp(1.0, 20_000.0);
        }
        if let Some(p) = num("powerPct") {
            self.power = p.clamp(0.0, 100.0);
        }
        if let Some(n) = num("passes") {
            self.passes = n.round().clamp(1.0, 20.0) as u32;
        }
    }

    fn to_json(self) -> Value {
        json!({"enabled": self.enabled, "speedMmMin": round1(self.speed), "powerPct": round1(self.power), "passes": self.passes})
    }
}

/// The three operations, in the table's order: cut, score, engrave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ops(pub [Op; 3]);

/// The operations' param names, in [`Ops`]' order.
pub const OP_KEYS: [&str; 3] = ["cut", "score", "engrave"];

/// An operation's label, translated.
fn op_label(key: &str) -> &'static str {
    match key {
        "cut" => tl!("Cut"),
        "score" => tl!("Score"),
        _ => tl!("Engrave"),
    }
}

impl Default for Ops {
    /// Paper's starting settings (as `vectorcraft-stencil`'s `GcodeParams::default`): shown only
    /// until the command reports the material's own.
    fn default() -> Self {
        Ops([Op::new(300.0, 100.0, 2), Op::new(1500.0, 30.0, 1), Op::new(2000.0, 20.0, 1)])
    }
}

impl Ops {
    /// Take the operations `v` (`{cut?, score?, engrave?}`, as a command's `gcode` result) sets
    /// validly; junk keeps what's there.
    pub fn read(&mut self, v: &Value) {
        for (key, op) in OP_KEYS.iter().zip(self.0.iter_mut()) {
            if let Some(o) = v.get(*key).filter(|o| o.is_object()) {
                op.read(o);
            }
        }
    }

    /// `{cut, score, engrave}` for a command's params.
    pub fn to_json(self) -> Value {
        let mut m = serde_json::Map::new();
        for (key, op) in OP_KEYS.iter().zip(self.0.iter()) {
            m.insert((*key).to_string(), op.to_json());
        }
        Value::Object(m)
    }

    /// Put the operations into a command's params `p` (an object).
    pub fn put(&self, p: &mut Value) {
        if let (Some(o), Value::Object(ops)) = (p.as_object_mut(), self.to_json()) {
            o.extend(ops);
        }
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Column widths: the operation's name, its switch, speed, power, passes.
const NAME_W: f32 = 62.0;
const ON_W: f32 = 22.0;
const SPEED_W: f32 = 104.0;
const POWER_W: f32 = 70.0;
const PASSES_W: f32 = 60.0;

/// The operation table; whether the person changed something.
pub fn table(ui: &mut Ui, salt: &str, ops: &mut Ops) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        for (text, w) in [("", NAME_W + ON_W), (tl!("Speed"), SPEED_W), (tl!("Power"), POWER_W), (tl!("Passes"), PASSES_W)] {
            ui.allocate_ui(egui::vec2(w, 16.0), |ui| {
                ui.set_min_width(w);
                ui.label(egui::RichText::new(text).size(10.5).color(t.text_dim));
            });
        }
    });
    for (key, op) in OP_KEYS.iter().zip(ops.0.iter_mut()) {
        let label = op_label(key);
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(NAME_W, 24.0), |ui| {
                ui.set_min_width(NAME_W);
                ui.label(egui::RichText::new(label).size(12.0).color(if op.enabled { t.text } else { t.text_disabled }));
            });
            ui.allocate_ui(egui::vec2(ON_W, 24.0), |ui| {
                ui.set_min_width(ON_W);
                if widgets::check_box(ui, label, op.enabled, true) {
                    op.enabled = !op.enabled;
                    changed = true;
                }
            });
            let id = |k: &str| ("picabia_laser_ops", salt.to_string(), *key, k.to_string());
            if let Some(v) = widgets::plain_field_named(ui, id("speed"), Some(tl!("Speed")), op.speed, "mm/min", 0, SPEED_W - 6.0)
                && v.is_finite()
            {
                op.speed = v.clamp(1.0, 20_000.0);
                changed = true;
            }
            if let Some(v) = widgets::plain_field_named(ui, id("power"), Some(tl!("Power")), op.power, "%", 1, POWER_W - 6.0)
                && v.is_finite()
            {
                op.power = v.clamp(0.0, 100.0);
                changed = true;
            }
            if let Some(v) = widgets::plain_field_named(ui, id("passes"), Some(tl!("Passes")), f64::from(op.passes), "×", 0, PASSES_W - 6.0)
                && v.is_finite()
            {
                op.passes = v.round().clamp(1.0, 20.0) as u32;
                changed = true;
            }
        });
    }
    changed
}

/// The material drop-down (`chosen` shown as its label); the material picked, when one was.
pub fn material_dropdown(ui: &mut Ui, salt: &str, chosen: &str, width: f32) -> Option<String> {
    let labels: Vec<&str> = MATERIALS.iter().map(|(_, l)| *l).collect();
    let current = MATERIALS.iter().find(|(id, _)| *id == chosen).map_or(chosen, |(_, l)| *l);
    let i = widgets::dropdown_named(ui, ("picabia_laser_material", salt), Some(tl!("Material")), current, &labels, width)?;
    MATERIALS.get(i).map(|(id, _)| (*id).to_string()).filter(|m| m != chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_read_what_is_valid_and_keep_the_rest() {
        let mut ops = Ops::default();
        ops.read(
            &json!({"cut": {"speedMmMin": 99999, "powerPct": "x", "passes": 2.6, "enabled": false}, "score": "junk", "engrave": {"powerPct": -5}}),
        );
        assert_eq!(ops.0[0], Op { enabled: false, speed: 20_000.0, power: 100.0, passes: 3 });
        assert_eq!(ops.0[1], Ops::default().0[1]);
        assert_eq!(ops.0[2].power, 0.0);
        let mut p = json!({"ids": [1]});
        ops.put(&mut p);
        assert_eq!(p["cut"]["passes"], 3);
        assert_eq!(p["ids"], json!([1]));
    }

    #[test]
    fn materials_have_labels() {
        assert_eq!(material_label("film"), "Film");
        assert_eq!(material_label("other"), "other");
    }
}
