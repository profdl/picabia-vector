//! The laser test card: a 120 × 80 mm sheet to cut once per material and measure, so the
//! material's real kerf and limits replace the presets' guesses.
//!
//! Rows (each labelled, every feature engraved with its value; a small triangle marks the one
//! nearest the material's current limit):
//! - **Bridge**: pairs of cut-outs leaving paper bridges 0.2 … 2.0 mm wide (marks: minimum
//!   bridge, tie width);
//! - **Hole**: round holes 0.3 … 3.0 mm across (mark: minimum cut width);
//! - **Slit**: slits 0.1 … 1.5 mm wide, 8 mm long (mark: minimum cut width);
//! - **Corner**: paper teeth with tip angles 15° … 90° (mark: minimum corner angle);
//! - **Kerf gauge**: a 20.000 mm square cut-out and a comb of 5 slots 20 mm outer to outer
//!   (measure the gap). The card is kerf-compensated with the kerf `k` it is made for, so the
//!   square's path is `20 − k` and its drop `20 − k − real kerf`: the engraved formula reads
//!   "kerf = (20 − k) − the drop's width" (plain "20 − width" at `k` = 0).
//!
//! The card goes through the same kerf, clean-up and cut order as any job.

use vectorcraft_geom::kurbo::{BezPath, Point, Rect};
use vectorcraft_modifiers::{MaterialLimits, StencilMaterial};

use crate::laser::{LaserParams, LaserSheet, Polyline, piece};
use crate::laser_font::text_polylines;
use crate::{hoshi, rings};

/// The card's size, mm (before kerf).
pub const CARD_W_MM: f64 = 120.0;
pub const CARD_H_MM: f64 = 80.0;
const MARGIN: f64 = 4.0;
/// Where the feature rows start (the row labels sit left of it).
const FEATURES_X: f64 = 22.0;
const VALUE_CAP: f64 = 1.5;
const LABEL_CAP: f64 = 1.8;

pub const BRIDGES_MM: [f64; 9] = [0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5, 2.0];
pub const HOLES_MM: [f64; 10] = [0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5, 2.0, 2.5, 3.0];
pub const SLITS_MM: [f64; 10] = [0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5];
pub const CORNERS_DEG: [f64; 6] = [15.0, 20.0, 30.0, 45.0, 60.0, 90.0];
/// The kerf gauge's square, mm.
pub const GAUGE_MM: f64 = 20.0;

struct Card {
    cut: Vec<Vec<Point>>,
    engrave: Vec<Polyline>,
}

impl Card {
    fn rect(&mut self, r: Rect) {
        self.cut.push(vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]);
    }

    fn text(&mut self, s: &str, origin: Point, cap: f64) -> f64 {
        let (lines, w) = text_polylines(s, origin, cap);
        self.engrave.extend(lines);
        w
    }

    /// `s` centred on `x`, baseline `y`.
    fn centred(&mut self, s: &str, x: f64, y: f64, cap: f64) {
        let w = text_polylines(s, Point::ZERO, cap).1;
        self.text(s, Point::new(x - w / 2.0, y), cap);
    }

    /// A small engraved triangle pointing up at `(x, y)`.
    fn mark(&mut self, x: f64, y: f64) {
        self.engrave.push(vec![Point::new(x, y), Point::new(x + 0.5, y + 0.9), Point::new(x - 0.5, y + 0.9), Point::new(x, y)]);
    }
}

/// The value of `values` nearest `target` (by index), if `target` is a number.
fn nearest(values: &[f64], target: f64) -> Option<usize> {
    if !target.is_finite() {
        return None;
    }
    values.iter().enumerate().min_by(|a, b| (a.1 - target).abs().total_cmp(&(b.1 - target).abs())).map(|(i, _)| i)
}

/// A value for a label: up to 2 decimals, trailing zeros trimmed, at least one decimal.
fn value(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0');
    if s.ends_with('.') { format!("{s}0") } else { s.to_string() }
}

/// A value with exactly 2 decimals.
fn value_fixed(v: f64) -> String {
    format!("{v:.2}")
}

/// `limits` with every non-finite or negative number replaced by the paper preset's.
fn sane(l: &MaterialLimits) -> MaterialLimits {
    let paper = StencilMaterial::Paper.limits().unwrap_or(MaterialLimits {
        min_cut_width_mm: 0.3,
        min_bridge_mm: 0.5,
        tie_width_mm: 1.0,
        min_corner_angle: 30.0,
        kerf_mm: 0.1,
    });
    let f = |v: f64, d: f64| if v.is_finite() && v >= 0.0 { v } else { d };
    MaterialLimits {
        min_cut_width_mm: f(l.min_cut_width_mm, paper.min_cut_width_mm),
        min_bridge_mm: f(l.min_bridge_mm, paper.min_bridge_mm),
        tie_width_mm: f(l.tie_width_mm, paper.tie_width_mm),
        min_corner_angle: f(l.min_corner_angle, paper.min_corner_angle),
        kerf_mm: f(l.kerf_mm, paper.kerf_mm),
    }
}

/// The test card for a material with `limits`, named `material_name` (engraved, at most 24
/// characters), cut with `kerf_mm` (0–2) compensation. Its name is `"test card"`.
pub fn test_card(limits: &MaterialLimits, material_name: &str, kerf_mm: f64) -> LaserSheet {
    let l = sane(limits);
    let k = if kerf_mm.is_finite() { kerf_mm.clamp(0.0, LaserParams::MAX_KERF_MM) } else { LaserParams::DEFAULT_KERF_MM };
    let mut c = Card { cut: Vec::new(), engrave: Vec::new() };
    let tolerance = 0.005;

    // Header.
    let name: String = material_name.trim().chars().take(24).collect::<String>().to_uppercase();
    let header = if name.is_empty() { format!("PICABIA TEST CARD - KERF {k:.2} MM") } else { format!("PICABIA TEST CARD - {name} - KERF {k:.2} MM") };
    let room = CARD_W_MM - 2.0 * MARGIN;
    let cap = [2.5, 2.0, 1.5].into_iter().find(|&cap| text_polylines(&header, Point::ZERO, cap).1 <= room).unwrap_or(1.2);
    c.text(&header, Point::new(MARGIN, MARGIN + 2.5), cap);

    // Bridges: pairs of 2.5 × 6 mm cut-outs.
    let (top, tall) = (11.0, 6.0);
    c.text("BRIDGE", Point::new(MARGIN, top + tall / 2.0 + LABEL_CAP / 2.0), LABEL_CAP);
    let marks = [nearest(&BRIDGES_MM, l.min_bridge_mm), nearest(&BRIDGES_MM, l.tie_width_mm)];
    let mut x = FEATURES_X;
    for (i, &w) in BRIDGES_MM.iter().enumerate() {
        c.rect(Rect::new(x, top, x + 2.5, top + tall));
        c.rect(Rect::new(x + 2.5 + w, top, x + 5.0 + w, top + tall));
        let mid = x + 2.5 + w / 2.0;
        let base = top + tall + 0.8 + VALUE_CAP;
        c.centred(&value(w), mid, base, VALUE_CAP);
        if marks.contains(&Some(i)) {
            c.mark(mid, base + 0.8);
        }
        x += 5.0 + w + 2.0;
    }

    // Holes.
    let top = 24.0;
    c.text("HOLE", Point::new(MARGIN, top + 1.5 + LABEL_CAP / 2.0), LABEL_CAP);
    let mark = nearest(&HOLES_MM, l.min_cut_width_mm);
    for (i, &d) in HOLES_MM.iter().enumerate() {
        let cx = FEATURES_X + 2.5 + 7.0 * i as f64;
        c.cut.push(hoshi::disc(Point::new(cx, top + 1.5), d / 2.0, tolerance));
        let base = top + 3.0 + 0.8 + VALUE_CAP;
        c.centred(&value(d), cx, base, VALUE_CAP);
        if mark == Some(i) {
            c.mark(cx, base + 0.8);
        }
    }

    // Slits, 8 mm long.
    let (top, long) = (34.0, 8.0);
    c.text("SLIT", Point::new(MARGIN, top + long / 2.0 + LABEL_CAP / 2.0), LABEL_CAP);
    let mark = nearest(&SLITS_MM, l.min_cut_width_mm);
    for (i, &w) in SLITS_MM.iter().enumerate() {
        let cx = FEATURES_X + 2.5 + 7.0 * i as f64;
        c.rect(Rect::new(cx - w / 2.0, top, cx + w / 2.0, top + long));
        let base = top + long + 0.8 + VALUE_CAP;
        c.centred(&value(w), cx, base, VALUE_CAP);
        if mark == Some(i) {
            c.mark(cx, base + 0.8);
        }
    }

    // Corners: 6 × 6 mm cut-outs with a paper tooth 2.8 mm tall rising from the bottom edge.
    let (top, size, tooth) = (49.0, 6.0, 2.8);
    c.text("CORNER", Point::new(MARGIN, top + size / 2.0 + LABEL_CAP / 2.0), LABEL_CAP);
    let mark = nearest(&CORNERS_DEG, l.min_corner_angle);
    for (i, &deg) in CORNERS_DEG.iter().enumerate() {
        let x0 = FEATURES_X + 9.0 * i as f64;
        let (x1, y0, y1) = (x0 + size, top, top + size);
        let cx = x0 + size / 2.0;
        let hw = (tooth * (deg / 2.0).to_radians().tan()).min(size / 2.0 - 0.2);
        c.cut.push(vec![
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(cx + hw, y1),
            Point::new(cx, y1 - tooth),
            Point::new(cx - hw, y1),
            Point::new(x0, y1),
        ]);
        let base = y1 + 0.8 + VALUE_CAP;
        c.centred(&format!("{deg:.0}°"), cx, base, VALUE_CAP);
        if mark == Some(i) {
            c.mark(cx, base + 0.8);
        }
    }

    // Kerf gauge: the square, then the comb.
    let gx = CARD_W_MM - MARGIN - 2.0 - GAUGE_MM;
    c.text("KERF", Point::new(gx, 9.8), VALUE_CAP);
    c.rect(Rect::new(gx, 11.0, gx + GAUGE_MM, 11.0 + GAUGE_MM));
    c.text("20.00", Point::new(gx, 11.0 + GAUGE_MM + 0.8 + VALUE_CAP), VALUE_CAP);
    let (slot, pitch) = (2.0, (GAUGE_MM - 2.0) / 4.0);
    for t in 0..5 {
        let sx = gx + pitch * f64::from(t);
        c.rect(Rect::new(sx, 37.0, sx + slot, 42.0));
    }
    c.text("COMB 20.00", Point::new(gx, 42.0 + 0.8 + VALUE_CAP), VALUE_CAP);

    // How to read it.
    // The square's path is 20 − k (compensated), so the drop is 20 − k − the real kerf.
    c.text(&format!("MEASURE THE DROP: KERF = {} - ITS WIDTH", value_fixed(GAUGE_MM - k)), Point::new(MARGIN, 65.0), LABEL_CAP);
    c.text("TRIANGLES MARK THE MATERIAL'S LIMITS", Point::new(MARGIN, 69.0), LABEL_CAP);

    let cut: BezPath = rings::to_path(&c.cut);
    let rect = Rect::new(0.0, 0.0, CARD_W_MM, CARD_H_MM);
    let shift = vectorcraft_geom::kurbo::Vec2::new(k / 2.0, k / 2.0);
    let contours = match piece(rect, &cut, k) {
        Ok(p) => p.contours,
        // The card's own shapes are fixed and finite; should the sweep still fail, the outline
        // alone keeps the sheet cuttable.
        Err(_) => vec![vec![Point::ZERO, Point::new(CARD_W_MM + k, 0.0), Point::new(CARD_W_MM + k, CARD_H_MM + k), Point::new(0.0, CARD_H_MM + k)]],
    };
    LaserSheet {
        w_mm: CARD_W_MM + k,
        h_mm: CARD_H_MM + k,
        cut: contours,
        score: Vec::new(),
        engrave: c.engrave.into_iter().map(|l| l.into_iter().map(|p| p + shift).collect()).collect(),
        name: "test card".into(),
        loose_pieces: 0,
    }
}
