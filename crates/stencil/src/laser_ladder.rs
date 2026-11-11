//! The power ladder: a grid of small squares, each cut at its own power and speed, so a diode's
//! cut setting for a material is found by looking at which squares dropped out cleanly.
//!
//! Columns are powers (left to right, labelled on top), rows are speeds (top to bottom, labelled
//! on the left), with a two-line header. The squares are cut contours; the labels are engraved.
//! The sheet's outline is **not** cut: the ladder is cut into a larger sheet of the material.
//! Nothing is kerf-compensated (the ladder measures power, not size).

use serde_json::{Map, Value};
use vectorcraft_geom::kurbo::Point;

use crate::laser::{LaserSheet, Polyline};
use crate::laser_font::text_polylines;
use crate::laser_gcode::{Block, Gcode, GcodeParams, OpSettings, op_line, write};
use crate::laser_svg::num;

/// The most powers (columns) and speeds (rows).
pub const MAX_LADDER_STEPS: usize = 12;
const MARGIN: f64 = 4.0;
const HEADER_CAP: f64 = 2.5;
const NOTE_CAP: f64 = 1.5;
const VALUE_CAP: f64 = 1.5;

/// Power ladder settings.
#[derive(Clone, Debug, PartialEq)]
pub struct LadderParams {
    /// Column powers, % (0–100), at most [`MAX_LADDER_STEPS`].
    pub powers_pct: Vec<f64>,
    /// Row speeds, mm/min (1–20000), at most [`MAX_LADDER_STEPS`].
    pub speeds_mm_min: Vec<f64>,
    /// The squares' side, mm (2–50).
    pub square_mm: f64,
    /// The paper between squares, mm (1–50).
    pub gap_mm: f64,
    /// Passes per square (1–20).
    pub passes: u32,
}

impl LadderParams {
    pub const MIN_SQUARE_MM: f64 = 2.0;
    pub const MAX_SQUARE_MM: f64 = 50.0;
    pub const MIN_GAP_MM: f64 = 1.0;
    pub const MAX_GAP_MM: f64 = 50.0;

    /// The settings in range: steps finite, clamped and at most [`MAX_LADDER_STEPS`] (none left:
    /// the defaults), sizes clamped (non-finite: the default).
    pub fn clamped(&self) -> LadderParams {
        let d = LadderParams::default();
        let steps = |v: &[f64], lo: f64, hi: f64, def: &[f64]| -> Vec<f64> {
            let s: Vec<f64> = v.iter().filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi)).take(MAX_LADDER_STEPS).collect();
            if s.is_empty() { def.to_vec() } else { s }
        };
        let size = |v: f64, lo: f64, hi: f64, def: f64| if v.is_finite() { v.clamp(lo, hi) } else { def };
        LadderParams {
            powers_pct: steps(&self.powers_pct, 0.0, 100.0, &d.powers_pct),
            speeds_mm_min: steps(&self.speeds_mm_min, OpSettings::MIN_SPEED, OpSettings::MAX_SPEED, &d.speeds_mm_min),
            square_mm: size(self.square_mm, Self::MIN_SQUARE_MM, Self::MAX_SQUARE_MM, d.square_mm),
            gap_mm: size(self.gap_mm, Self::MIN_GAP_MM, Self::MAX_GAP_MM, d.gap_mm),
            passes: self.passes.clamp(1, OpSettings::MAX_PASSES),
        }
    }

    /// Settings from a JSON object, leniently (see [`apply_json`](Self::apply_json)).
    pub fn from_json(v: &Value) -> LadderParams {
        let mut p = LadderParams::default();
        p.apply_json(v);
        p
    }

    /// Overwrite the fields `v` sets validly (camelCase: `powersPct`, `speedsMmMin` (arrays of
    /// numbers; other entries skipped), `squareMm`, `gapMm`, `passes`), then clamp.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        let list =
            |k: &str| o.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).filter(|x| x.is_finite()).collect::<Vec<f64>>());
        if let Some(l) = list("powersPct") {
            self.powers_pct = l;
        }
        if let Some(l) = list("speedsMmMin") {
            self.speeds_mm_min = l;
        }
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(x) = num("squareMm") {
            self.square_mm = x;
        }
        if let Some(x) = num("gapMm") {
            self.gap_mm = x;
        }
        if let Some(x) = num("passes") {
            self.passes = x.round().clamp(1.0, f64::from(OpSettings::MAX_PASSES)) as u32;
        }
        *self = self.clamped();
    }

    /// The settings as JSON (the [`apply_json`](Self::apply_json) names), clamped.
    pub fn to_json(&self) -> Value {
        let p = self.clamped();
        let mut o = Map::new();
        o.insert("powersPct".into(), Value::from(p.powers_pct));
        o.insert("speedsMmMin".into(), Value::from(p.speeds_mm_min));
        o.insert("squareMm".into(), Value::from(p.square_mm));
        o.insert("gapMm".into(), Value::from(p.gap_mm));
        o.insert("passes".into(), Value::from(p.passes));
        Value::Object(o)
    }
}

impl Default for LadderParams {
    fn default() -> Self {
        LadderParams {
            powers_pct: vec![20.0, 40.0, 60.0, 80.0, 100.0],
            speeds_mm_min: vec![150.0, 300.0, 600.0, 1200.0],
            square_mm: 6.0,
            gap_mm: 4.0,
            passes: 1,
        }
    }
}

/// One square of the ladder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderCell {
    /// Its contour in the sheet's `cut`.
    pub contour: usize,
    pub power_pct: f64,
    pub speed_mm_min: f64,
    pub passes: u32,
}

/// The ladder: its sheet (`"power ladder"`; squares as cut contours row by row, top row first,
/// left to right; labels engraved) and which setting each square is cut at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ladder {
    pub sheet: LaserSheet,
    pub cells: Vec<LadderCell>,
}

/// A step's label: up to 1 decimal, trailing zeros trimmed.
fn step_label(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The power ladder for `params` (clamped first). See the module docs.
pub fn power_ladder(params: &LadderParams) -> Ladder {
    let p = params.clamped();
    let (cols, rows) = (p.powers_pct.len(), p.speeds_mm_min.len());
    let (sq, gap) = (p.square_mm, p.gap_mm);
    let mut engrave: Vec<Polyline> = Vec::new();
    let mut text = |s: &str, origin: Point, cap: f64| {
        let (lines, w) = text_polylines(s, origin, cap);
        engrave.extend(lines);
        w
    };
    let width = |s: &str, cap: f64| text_polylines(s, Point::ZERO, cap).1;

    // Header: two lines.
    let title = "PICABIA POWER LADDER";
    let note = format!("POWER % ACROSS, MM/MIN DOWN, {} PASS{}", p.passes, if p.passes == 1 { "" } else { "ES" });
    let title_y = MARGIN + HEADER_CAP;
    let note_y = title_y + 1.2 + NOTE_CAP;
    text(title, Point::new(MARGIN, title_y), HEADER_CAP);
    text(&note, Point::new(MARGIN, note_y), NOTE_CAP);
    let header_w = width(title, HEADER_CAP).max(width(&note, NOTE_CAP));

    // Columns: power labels above the grid; rows: speed labels left of it.
    let speed_labels: Vec<String> = p.speeds_mm_min.iter().map(|&v| step_label(v)).collect();
    let label_w = speed_labels.iter().map(|s| width(s, VALUE_CAP)).fold(0.0, f64::max);
    let gx = MARGIN + label_w + 2.0;
    let power_y = note_y + 2.0 + VALUE_CAP;
    let gy = power_y + 1.2;
    let pitch = sq + gap;
    for (i, &pw) in p.powers_pct.iter().enumerate() {
        let s = step_label(pw);
        let cx = gx + i as f64 * pitch + sq / 2.0;
        text(&s, Point::new(cx - width(&s, VALUE_CAP) / 2.0, power_y), VALUE_CAP);
    }
    let mut cut = Vec::with_capacity(cols * rows);
    let mut cells = Vec::with_capacity(cols * rows);
    for (j, (&speed, label)) in p.speeds_mm_min.iter().zip(&speed_labels).enumerate() {
        let y = gy + j as f64 * pitch;
        text(label, Point::new(gx - 2.0 - width(label, VALUE_CAP), y + sq / 2.0 + VALUE_CAP / 2.0), VALUE_CAP);
        for (i, &power) in p.powers_pct.iter().enumerate() {
            let x = gx + i as f64 * pitch;
            cells.push(LadderCell { contour: cut.len(), power_pct: power, speed_mm_min: speed, passes: p.passes });
            cut.push(vec![Point::new(x, y), Point::new(x + sq, y), Point::new(x + sq, y + sq), Point::new(x, y + sq)]);
        }
    }
    let grid_w = cols as f64 * pitch - gap;
    let grid_h = rows as f64 * pitch - gap;
    let w = (gx + grid_w).max(MARGIN + header_w) + MARGIN;
    let h = gy + grid_h + MARGIN;
    Ladder { sheet: LaserSheet { w_mm: w, h_mm: h, cut, score: Vec::new(), engrave, name: "power ladder".into(), loose_pieces: 0 }, cells }
}

/// The ladder's G-code: the labels engraved with `params.engrave` first, then each square cut
/// at its cell's power, speed and passes (`M4 S…` … `M5` per square); `params.cut` and
/// `params.score` are not used, the sheet's outline is not cut. The rest of `params` (S max,
/// origin, air, header) as for [`laser_gcode`](crate::laser_gcode).
pub fn ladder_gcode(ladder: &Ladder, params: &GcodeParams) -> Gcode {
    let p = params.clamped();
    // PICABIA: the ladder measures the beam's power; a knife has none.
    if p.head == crate::Head::Knife {
        return Gcode { warnings: vec!["the power ladder is for the laser: the head is the drag knife".into()], ..Gcode::default() };
    }
    let sheet = &ladder.sheet;
    let mut blocks = vec![Block { op: p.engrave, paths: sheet.engrave.iter().map(Vec::as_slice).collect(), closed: false }];
    for cell in &ladder.cells {
        let Some(contour) = sheet.cut.get(cell.contour) else { continue };
        let op = OpSettings { enabled: true, speed_mm_min: cell.speed_mm_min, power_pct: cell.power_pct, passes: cell.passes }.clamped_or(&p.cut);
        blocks.push(Block { op, paths: vec![contour.as_slice()], closed: true });
    }
    let info = vec![
        format!("S max {}, M4 dynamic power", num(p.s_max)),
        op_line("engrave", p.engrave, p.s_max),
        format!("power ladder: {} squares, each cut at its own power and speed", ladder.cells.len()),
    ];
    write(sheet, &blocks, &p, &info)
}
