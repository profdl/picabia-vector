//! The power ladder: cells, layout, labels, G-code, params.

use serde_json::json;
use vectorcraft_geom::kurbo::Rect;

use crate::laser::bounds;
use crate::{GcodeParams, LadderParams, MAX_LADDER_STEPS, ladder_gcode, power_ladder};

fn code(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.starts_with(';')).collect()
}

fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace().find_map(|w| w.strip_prefix(letter)).and_then(|v| v.parse().ok())
}

#[test]
fn the_ladder_has_a_square_per_setting() {
    let l = power_ladder(&LadderParams::default());
    let s = &l.sheet;
    assert_eq!(s.name, "power ladder");
    assert_eq!(l.cells.len(), 20);
    assert_eq!(s.cut.len(), 20, "squares only: the outline is not cut");
    assert!(s.score.is_empty());
    // Row by row (speeds), left to right (powers).
    let powers: Vec<f64> = l.cells.iter().take(5).map(|c| c.power_pct).collect();
    assert_eq!(powers, vec![20.0, 40.0, 60.0, 80.0, 100.0]);
    let speeds: Vec<f64> = l.cells.iter().step_by(5).map(|c| c.speed_mm_min).collect();
    assert_eq!(speeds, vec![150.0, 300.0, 600.0, 1200.0]);
    let page = Rect::new(0.0, 0.0, s.w_mm, s.h_mm);
    let mut boxes = Vec::new();
    for (k, c) in l.cells.iter().enumerate() {
        assert_eq!(c.contour, k);
        assert_eq!(c.passes, 1);
        let b = bounds(&s.cut[c.contour]);
        assert!((b.width() - 6.0).abs() < 1e-9 && (b.height() - 6.0).abs() < 1e-9, "{b:?}");
        assert!(b.x0 >= 4.0 && b.y0 >= 4.0 && b.x1 <= s.w_mm - 4.0 && b.y1 <= s.h_mm - 4.0, "inside the margin: {b:?}");
        boxes.push(b);
    }
    // Columns and rows 10 mm apart (6 + 4).
    assert!((boxes[1].x0 - boxes[0].x0 - 10.0).abs() < 1e-9);
    assert!((boxes[5].y0 - boxes[0].y0 - 10.0).abs() < 1e-9);
    // Labels: above the grid (header and powers), left of it (speeds); none on a square.
    let grid = boxes.iter().fold(boxes[0], |a, b| a.union(*b));
    assert!(!s.engrave.is_empty());
    for line in &s.engrave {
        let b = bounds(line);
        assert!(page.contains(b.origin()) && b.x1 <= s.w_mm && b.y1 <= s.h_mm, "{b:?}");
        assert!(b.y1 <= grid.y0 || b.x1 <= grid.x0, "label clear of the grid: {b:?}");
    }
    let above_columns = |i: usize| {
        s.engrave
            .iter()
            .filter(|p| bounds(p).y1 <= grid.y0 && bounds(p).y0 > grid.y0 - 4.0)
            .any(|p| bounds(p).x0 >= boxes[i].x0 - 1.0 && bounds(p).x1 <= boxes[i].x1 + 1.0)
    };
    assert!((0..5).all(above_columns), "a power label over every column");
    let left_rows =
        |j: usize| s.engrave.iter().filter(|p| bounds(p).x1 <= grid.x0).any(|p| bounds(p).y0 >= boxes[j * 5].y0 && bounds(p).y1 <= boxes[j * 5].y1);
    assert!((0..4).all(left_rows), "a speed label by every row");
}

#[test]
fn every_square_is_cut_at_its_own_setting() {
    let l = power_ladder(&LadderParams { passes: 2, ..LadderParams::default() });
    let p = GcodeParams { s_max: 1000.0, ..GcodeParams::default() };
    let g = ladder_gcode(&l, &p);
    assert!(!g.text.is_empty(), "{:?}", g.warnings);
    let c = code(&g.text);
    // Labels first at the engrave setting, then one M4 … M5 block per square.
    let s: Vec<f64> = c.iter().filter(|l| l.starts_with("M4")).filter_map(|l| word(l, 'S')).collect();
    let mut want = vec![200.0];
    for _ in 0..4 {
        want.extend([200.0, 400.0, 600.0, 800.0, 1000.0]);
    }
    assert_eq!(s, want);
    let f: Vec<f64> = c.iter().filter_map(|l| word(l, 'F')).collect();
    let mut want = vec![2000.0];
    for v in [150.0, 300.0, 600.0, 1200.0] {
        want.extend([v; 5]);
    }
    assert_eq!(f, want);
    // Each square: one G0, 2 passes × 4 sides.
    let blocks: Vec<&[&str]> = c.split(|l| *l == "M5").collect();
    for b in blocks.iter().filter(|b| b.first().is_some_and(|l| l.starts_with("M4"))).skip(1) {
        assert_eq!(b.iter().filter(|l| l.starts_with("G0")).count(), 1);
        assert_eq!(b.iter().filter(|l| l.starts_with("G1")).count(), 8);
    }
    // Beam off at both ends; the outline never cut: every motion inside the page.
    assert_eq!(c[2], "M5");
    assert_eq!(c[c.len() - 1], "M5");
    assert!(c[c.len() - 2].starts_with("G0 X0 Y0"));
    let b = g.bounds_mm;
    assert!(b.x1 < l.sheet.w_mm && b.y1 < l.sheet.h_mm && b.x0 == 0.0 && b.y0 == 0.0, "{b:?}");
    assert!(g.text.contains("; power ladder: 20 squares"));
    // The labels follow the engrave switch; the squares don't follow cut's.
    let mut q = p.clone();
    q.engrave.enabled = false;
    q.cut.enabled = false;
    let g = ladder_gcode(&l, &q);
    assert_eq!(code(&g.text).iter().filter(|l| l.starts_with("M4")).count(), 20);
    assert_eq!(ladder_gcode(&l, &p), ladder_gcode(&l, &p), "deterministic");
}

#[test]
fn ladder_params_clamp_and_read_json_leniently() {
    let d = LadderParams::default();
    assert_eq!(d.clamped(), d);
    let big = LadderParams {
        powers_pct: (0..30).map(f64::from).chain([f64::NAN]).collect(),
        speeds_mm_min: vec![f64::INFINITY, -5.0, 1e9],
        square_mm: 0.0,
        gap_mm: f64::NAN,
        passes: 0,
    }
    .clamped();
    assert_eq!(big.powers_pct.len(), MAX_LADDER_STEPS);
    assert_eq!(big.speeds_mm_min, vec![1.0, 20_000.0]);
    assert_eq!((big.square_mm, big.gap_mm, big.passes), (2.0, 4.0, 1));
    let l = power_ladder(&big);
    assert_eq!(l.cells.len(), MAX_LADDER_STEPS * 2);
    assert!(l.sheet.w_mm.is_finite() && l.sheet.h_mm.is_finite());
    let empty = LadderParams { powers_pct: vec![f64::NAN], speeds_mm_min: Vec::new(), ..d.clone() }.clamped();
    assert_eq!((empty.powers_pct, empty.speeds_mm_min), (d.powers_pct.clone(), d.speeds_mm_min.clone()));
    let j =
        LadderParams::from_json(&json!({"powersPct": [10, "x", 50, null, 250], "speedsMmMin": "fast", "squareMm": 8, "gapMm": 99, "passes": 3.4}));
    assert_eq!(j.powers_pct, vec![10.0, 50.0, 100.0]);
    assert_eq!(j.speeds_mm_min, d.speeds_mm_min);
    assert_eq!((j.square_mm, j.gap_mm, j.passes), (8.0, 50.0, 3));
    assert_eq!(LadderParams::from_json(&j.to_json()), j);
    for junk in [json!(null), json!(1), json!([]), json!({"powersPct": []})] {
        assert_eq!(LadderParams::from_json(&junk), d);
    }
    // A single fractional step labels with one decimal.
    let one = power_ladder(&LadderParams { powers_pct: vec![12.5], speeds_mm_min: vec![333.3], ..d });
    assert_eq!(one.cells.len(), 1);
    let g = ladder_gcode(&one, &GcodeParams { s_max: 255.0, ..GcodeParams::default() });
    assert!(code(&g.text).contains(&"M4 S32"), "12.5 % of 255 = 31.875");
    assert!(g.text.lines().all(|l| l.len() <= 80 && l.is_ascii()));
}
