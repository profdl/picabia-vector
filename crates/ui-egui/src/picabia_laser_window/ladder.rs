//! PICABIA: the power ladder's grid in the Laser window: the range of powers (columns) and speeds
//! (rows) and how many steps each, sent as the command's `powersPct` and `speedsMmMin`. Powers are
//! spaced evenly; speeds by an even ratio (150 to 1200 in 4 steps: 150, 300, 600, 1200), since a
//! cut changes as much from 150 to 300 mm/min as from 600 to 1200.

use egui::Ui;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::widgets;

/// The most steps per axis (as `vectorcraft-stencil`'s `MAX_LADDER_STEPS`).
pub const MAX_STEPS: u32 = 12;
const MIN_SPEED: f64 = 1.0;
const MAX_SPEED: f64 = 20_000.0;

/// One axis of the grid: from, to and how many steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub from: f64,
    pub to: f64,
    /// 1-[`MAX_STEPS`].
    pub steps: u32,
}

/// The ladder's powers (%) and speeds (mm/min).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub power: Range,
    pub speed: Range,
}

impl Default for Grid {
    /// The command's own default grid: 20-100 % in 5 steps, 150-1200 mm/min in 4.
    fn default() -> Self {
        Grid { power: Range { from: 20.0, to: 100.0, steps: 5 }, speed: Range { from: 150.0, to: 1200.0, steps: 4 } }
    }
}

impl Grid {
    /// The column powers, %: evenly spaced from `from` to `to`, to 0.1.
    pub fn powers(&self) -> Vec<f64> {
        let Range { from, to, steps } = self.power;
        let n = steps.clamp(1, MAX_STEPS);
        (0..n).map(|i| round1(if n == 1 { from } else { from + (to - from) * f64::from(i) / f64::from(n - 1) })).collect()
    }

    /// The row speeds, mm/min: an even ratio from `from` to `to`, whole.
    pub fn speeds(&self) -> Vec<f64> {
        let Range { from, to, steps } = self.speed;
        let n = steps.clamp(1, MAX_STEPS);
        let (a, b) = (from.clamp(MIN_SPEED, MAX_SPEED), to.clamp(MIN_SPEED, MAX_SPEED));
        (0..n).map(|i| if n == 1 { a.round() } else { (a * (b / a).powf(f64::from(i) / f64::from(n - 1))).round() }).collect()
    }

    /// Put `powersPct` and `speedsMmMin` into a command's params `p` (an object).
    pub fn put(&self, p: &mut Value) {
        if let Some(o) = p.as_object_mut() {
            o.insert("powersPct".into(), json!(self.powers()));
            o.insert("speedsMmMin".into(), json!(self.speeds()));
        }
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Column widths: the axis's name, from, to, steps.
const NAME_W: f32 = 84.0;
const FIELD_W: f32 = 96.0;
const STEPS_W: f32 = 64.0;

/// The grid's fields; whether the person changed something.
pub fn table(ui: &mut Ui, g: &mut Grid) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        for (text, w) in [("", NAME_W), (tl!("From"), FIELD_W), (tl!("To"), FIELD_W), (tl!("Steps"), STEPS_W)] {
            ui.allocate_ui(egui::vec2(w, 16.0), |ui| {
                ui.set_min_width(w);
                ui.label(egui::RichText::new(text).size(10.5).color(t.text_dim));
            });
        }
    });
    let rows: [(&str, &str, &mut Range, usize, f64, f64); 2] =
        [("power", tl!("Power"), &mut g.power, 1, 0.0, 100.0), ("speed", tl!("Speed"), &mut g.speed, 0, MIN_SPEED, MAX_SPEED)];
    for (key, label, r, decimals, lo, hi) in rows {
        let suffix = if key == "power" { "%" } else { "mm/min" };
        ui.horizontal(|ui| {
            super::lead(ui, label);
            let id = |k: &str| ("picabia_laser_ladder", key, k.to_string());
            if let Some(v) = widgets::plain_field_named(ui, id("from"), Some(tl!("From")), r.from, suffix, decimals, FIELD_W - 6.0)
                && v.is_finite()
            {
                r.from = v.clamp(lo, hi);
                changed = true;
            }
            if let Some(v) = widgets::plain_field_named(ui, id("to"), Some(tl!("To")), r.to, suffix, decimals, FIELD_W - 6.0)
                && v.is_finite()
            {
                r.to = v.clamp(lo, hi);
                changed = true;
            }
            if let Some(v) = widgets::plain_field_named(ui, id("steps"), Some(tl!("Steps")), f64::from(r.steps), "", 0, STEPS_W - 6.0)
                && v.is_finite()
            {
                r.steps = v.round().clamp(1.0, f64::from(MAX_STEPS)) as u32;
                changed = true;
            }
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_grid_is_the_commands() {
        let g = Grid::default();
        assert_eq!(g.powers(), vec![20.0, 40.0, 60.0, 80.0, 100.0]);
        assert_eq!(g.speeds(), vec![150.0, 300.0, 600.0, 1200.0]);
    }

    #[test]
    fn ranges_space_their_steps() {
        let g = Grid { power: Range { from: 10.0, to: 30.0, steps: 3 }, speed: Range { from: 2000.0, to: 500.0, steps: 3 } };
        assert_eq!(g.powers(), vec![10.0, 20.0, 30.0]);
        assert_eq!(g.speeds(), vec![2000.0, 1000.0, 500.0], "from may be above to");
        let one = Grid { power: Range { from: 55.0, to: 90.0, steps: 1 }, speed: Range { from: 800.0, to: 100.0, steps: 1 } };
        assert_eq!(one.powers(), vec![55.0]);
        assert_eq!(one.speeds(), vec![800.0]);
        let many = Grid { power: Range { steps: 99, ..g.power }, speed: Range { from: 0.0, to: 1e9, steps: 0 } };
        assert_eq!(many.powers().len(), MAX_STEPS as usize);
        assert_eq!(many.speeds(), vec![MIN_SPEED], "clamped");
        let mut p = json!({"testCard": "powerLadder"});
        g.put(&mut p);
        assert_eq!(p["powersPct"], json!([10.0, 20.0, 30.0]));
        assert_eq!(p["testCard"], "powerLadder");
    }
}
