//! G-code export: coordinates, power, passes, order, beam-off ends, words, limits, params.

use std::time::Instant;

use serde_json::json;
use vectorcraft_geom::kurbo::{BezPath, Circle, Point, Rect, Shape};

use vectorcraft_modifiers::StencilMaterial;

use crate::fixture::Rng;
use crate::{Gcode, GcodeParams, LaserInput, LaserParams, LaserSheet, MAX_GCODE_LINES, OpSettings, estimate_sec, laser_gcode, laser_job, test_card};

fn pts(v: &[(f64, f64)]) -> Vec<Point> {
    v.iter().map(|&(x, y)| Point::new(x, y)).collect()
}

/// A `w` × `h` sheet with the given cut contours and nothing else.
fn sheet(w: f64, h: f64, cut: Vec<Vec<Point>>) -> LaserSheet {
    LaserSheet { w_mm: w, h_mm: h, cut, name: "1 of 1".into(), ..LaserSheet::default() }
}

fn square(x: f64, y: f64, s: f64) -> Vec<Point> {
    pts(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)])
}

/// The non-comment lines.
fn code(g: &Gcode) -> Vec<&str> {
    g.text.lines().filter(|l| !l.starts_with(';')).collect()
}

/// A word's value on a line (`X`, `Y`, `F`, `S`).
fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace().find_map(|w| w.strip_prefix(letter)).and_then(|v| v.parse().ok())
}

/// Every motion: (G number, x, y).
fn moves(g: &Gcode) -> Vec<(u8, f64, f64)> {
    code(g)
        .into_iter()
        .filter_map(|l| {
            let gn = if l.starts_with("G0 ") {
                0
            } else if l.starts_with("G1 ") {
                1
            } else {
                return None;
            };
            Some((gn, word(l, 'X')?, word(l, 'Y')?))
        })
        .collect()
}

fn s_values(g: &Gcode) -> Vec<f64> {
    code(g).into_iter().filter(|l| l.starts_with("M4")).filter_map(|l| word(l, 'S')).collect()
}

/// The checks every file must pass: beam off at both ends, `G1` only between `M4` and `M5`,
/// only the allowed words, ASCII lines of at most 80 bytes, no exponents, LF endings.
fn assert_safe(g: &Gcode) {
    assert!(!g.text.is_empty(), "{:?}", g.warnings);
    assert!(!g.text.contains('\r'));
    assert_eq!(g.lines, g.text.lines().count());
    for l in g.text.lines() {
        assert!(l.len() <= 80 && l.is_ascii(), "{l}");
        if l.starts_with(';') {
            assert!(l.len() <= 70, "header line {l}");
            continue;
        }
        for w in l.split_whitespace() {
            let (letter, value) = w.split_at(1);
            assert!(!value.contains(['e', 'E']), "exponent in {l}");
            assert!(value != "-0" && !value.starts_with("-0.000") && value.parse::<f64>().is_ok(), "number {w} in {l}");
            match letter {
                "G" => assert!(["0", "1", "21", "90"].contains(&value), "{l}"),
                "M" => assert!(["4", "5", "8", "9"].contains(&value), "{l}"),
                "S" | "F" | "X" | "Y" => {}
                _ => panic!("word {w} in {l}"),
            }
        }
    }
    let c = code(g);
    // Beam off before the first motion; the last lines are M5, the park, M5.
    let first_motion = c.iter().position(|l| l.starts_with("G0") || l.starts_with("G1")).unwrap();
    assert!(c[..first_motion].contains(&"M5"), "M5 before the first motion");
    assert!(c[first_motion].starts_with("G0"), "the first motion is a travel");
    let n = c.len();
    assert_eq!(c[n - 1], "M5");
    assert!(c[n - 2].starts_with("G0 "), "park: {}", c[n - 2]);
    assert!(c[..n - 2].iter().rev().find(|l| !l.starts_with("M9")).is_some_and(|l| *l == "M5"));
    // G1 only under M4; every M4 is turned off.
    let mut on = false;
    for l in &c {
        if l.starts_with("M4") {
            assert!(!on, "M4 twice without M5");
            on = true;
        } else if *l == "M5" {
            on = false;
        } else if l.starts_with("G1") {
            assert!(on, "G1 with the beam off: {l}");
        }
    }
    assert!(!on);
}

#[test]
fn coordinates_are_flipped_and_offset() {
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let p = GcodeParams { origin_mm: Point::new(5.0, 7.0), ..GcodeParams::default() };
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    let m = moves(&g);
    // G0 to the start: x 10 + 5, y 7 + (50 − 10).
    assert_eq!(m[0], (0, 15.0, 47.0));
    assert_eq!(m[1], (1, 25.0, 47.0));
    assert_eq!(m[2], (1, 25.0, 37.0));
    assert_eq!(m[3], (1, 15.0, 37.0));
    assert_eq!(m[4], (1, 15.0, 47.0), "closed: back to the start");
    assert_eq!(*m.last().unwrap(), (0, 5.0, 7.0), "park at the origin");
    assert_eq!(g.bounds_mm, Rect::new(5.0, 7.0, 25.0, 47.0));
    // Two passes of 40 mm at 300 mm/min.
    assert!((g.burn_mm - 80.0).abs() < 1e-9);
    assert!((g.burn_sec - 16.0).abs() < 1e-9);
    let travel = (10f64.hypot(40.0)) * 2.0;
    assert!((g.travel_mm - travel).abs() < 1e-9, "{} vs {travel}", g.travel_mm);
    assert!((estimate_sec(&g, 3000.0) - (16.0 + travel / 50.0)).abs() < 1e-9);
    assert!((estimate_sec(&g, f64::NAN) - (16.0 + travel * 60.0)).abs() < 1e-6, "rapid at least 1 mm/min");
    // Only the first G1 of the op carries F.
    let f: Vec<&str> = code(&g).into_iter().filter(|l| l.contains('F')).collect();
    assert_eq!(f, vec!["G1 X25 Y47 F300"]);
}

#[test]
fn power_scales_to_s_max_and_rounds() {
    let mut p = GcodeParams::default();
    p.cut.power_pct = 33.33;
    let s = sheet(50.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    assert_eq!(s_values(&laser_gcode(&s, &p)), vec![333.0]);
    p.s_max = 255.0;
    p.cut.power_pct = 50.0;
    assert_eq!(s_values(&laser_gcode(&s, &p)), vec![128.0], "127.5 rounds up");
    p.s_max = 12000.0;
    p.cut.power_pct = 0.0;
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    assert_eq!(s_values(&g), vec![0.0], "power 0 is an air run");
    assert!(g.text.contains("S max 12000"));
}

#[test]
fn passes_repeat_in_place() {
    let mut s = sheet(100.0, 100.0, vec![square(10.0, 10.0, 10.0), square(50.0, 50.0, 10.0)]);
    s.engrave = vec![pts(&[(1.0, 1.0), (5.0, 1.0), (5.0, 3.0)])];
    let mut p = GcodeParams::default();
    p.cut.passes = 3;
    p.engrave.passes = 2;
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    let m = moves(&g);
    let kinds: String = m.iter().map(|k| if k.0 == 0 { 'r' } else { 'b' }).collect();
    // Engrave: travel, 2 burns, back, 2 burns. Each square: travel, 3 × 4 burns. Park.
    let square_run = format!("r{}", "b".repeat(12));
    assert_eq!(kinds, format!("rbbrbb{square_run}{square_run}r"));
    assert!((g.burn_mm - (2.0 * 6.0 + 2.0 * 3.0 * 40.0)).abs() < 1e-9);
}

#[test]
fn operations_run_engrave_score_cut_and_disabled_ones_are_left_out() {
    let mut s = sheet(100.0, 100.0, vec![square(10.0, 10.0, 10.0)]);
    s.engrave = vec![pts(&[(1.0, 1.0), (5.0, 1.0)])];
    s.score = vec![pts(&[(1.0, 50.0), (99.0, 50.0)])];
    let p = GcodeParams::default();
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    assert_eq!(s_values(&g), vec![200.0, 300.0, 1000.0], "engrave 20 %, score 30 %, cut 100 %");
    let feeds: Vec<f64> = code(&g).into_iter().filter_map(|l| word(l, 'F')).collect();
    assert_eq!(feeds, vec![2000.0, 1500.0, 300.0]);
    let mut q = p.clone();
    q.score.enabled = false;
    q.engrave.enabled = false;
    let g = laser_gcode(&s, &q);
    assert_safe(&g);
    assert_eq!(s_values(&g), vec![1000.0]);
    assert!(g.text.contains("; engrave: off") && g.text.contains("; score: off"));
    // An enabled op with nothing to draw is left out too.
    let g = laser_gcode(&sheet(100.0, 100.0, vec![square(10.0, 10.0, 10.0)]), &p);
    assert_eq!(s_values(&g), vec![1000.0]);
    // Nothing left: refused.
    q.cut.enabled = false;
    let g = laser_gcode(&s, &q);
    assert!(g.text.is_empty() && g.lines == 0 && !g.warnings.is_empty());
    let g = laser_gcode(&sheet(10.0, 10.0, Vec::new()), &p);
    assert!(g.text.is_empty() && !g.warnings.is_empty());
}

#[test]
fn air_assist_wraps_the_job() {
    let s = sheet(50.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let g = laser_gcode(&s, &GcodeParams { air: true, ..GcodeParams::default() });
    assert_safe(&g);
    let c = code(&g);
    assert_eq!(&c[..4], &["G21", "G90", "M5", "M8"]);
    let n = c.len();
    assert_eq!(&c[n - 4..n - 2], &["M5", "M9"]);
    let off = laser_gcode(&s, &GcodeParams::default());
    assert!(!off.text.contains("M8") && !off.text.contains("M9"));
    assert_eq!(&code(&off)[..3], &["G21", "G90", "M5"]);
}

#[test]
fn numbers_have_no_exponent_or_negative_zero() {
    let s = sheet(50.0, 50.0, vec![pts(&[(-0.0001, 1e-9), (20.123456, 1e-7), (20.0, 49.99999), (3e-12, 50.0004)])]);
    let p = GcodeParams { origin_mm: Point::new(-0.0002, 1e-10), ..GcodeParams::default() };
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    let m = moves(&g);
    assert_eq!(m[0], (0, 0.0, 50.0));
    assert_eq!(m[1], (1, 20.123, 50.0));
    assert!(code(&g).iter().all(|l| !l.contains("-0 ") && !l.ends_with("-0")));
}

#[test]
fn invalid_points_are_skipped_with_a_warning() {
    let s = sheet(
        50.0,
        50.0,
        vec![pts(&[(1.0, 1.0), (f64::NAN, 2.0), (10.0, 1.0), (10.0, f64::INFINITY), (10.0, 10.0), (1e30, 0.0)]), pts(&[(f64::NAN, 1.0), (2.0, 2.0)])],
    );
    let g = laser_gcode(&s, &GcodeParams::default());
    assert_safe(&g);
    assert_eq!(g.warnings, vec!["4 points with invalid coordinates were left out".to_string()]);
    assert_eq!(moves(&g).len(), 1 + 2 * 3 + 1, "one contour of 3 points, 2 passes, park");
    // A sheet without a valid size is refused.
    for (w, h) in [(f64::NAN, 1.0), (1.0, f64::INFINITY), (-1.0, 5.0), (1e9, 1.0)] {
        let mut bad = s.clone();
        (bad.w_mm, bad.h_mm) = (w, h);
        let g = laser_gcode(&bad, &GcodeParams::default());
        assert!(g.text.is_empty() && !g.warnings.is_empty());
    }
}

#[test]
fn the_header_is_short_ascii_comments() {
    let mut s = sheet(230.1, 150.1, vec![square(10.0, 10.0, 10.0)]);
    s.name = "Fleur — 1 of 2".into();
    let mut header = vec!["Layer: Ünïcode ✂".into(), "x".repeat(500), "line\nbreak\r".into()];
    header.extend((0..40).map(|i| format!("extra {i}")));
    let p = GcodeParams { header, ..GcodeParams::default() };
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    let header: Vec<&str> = g.text.lines().take_while(|l| l.starts_with(';')).collect();
    assert!(header[0].contains("Picabia"));
    assert!(header.contains(&"; sheet: Fleur ? 1 of 2"));
    assert!(header.contains(&"; size: 230.1 x 150.1 mm"));
    assert!(header.contains(&"; cut: 300 mm/min, 100% (S1000), 2 passes"));
    assert!(header.contains(&"; score: 1500 mm/min, 30% (S300), 1 pass"));
    assert!(header.contains(&"; Layer: ?n?code ?"));
    assert!(header.contains(&"; line?break?"));
    assert!(!g.text.contains("extra 13") && g.text.contains("extra 12"), "16 caller lines at most");
    assert_eq!(g.text.lines().filter(|l| l.starts_with(';')).count(), header.len(), "comments only at the top");
}

#[test]
fn output_is_deterministic() {
    let mut s = sheet(120.0, 80.0, (0..30).map(|i| square(f64::from(i % 6) * 15.0 + 3.0, f64::from(i / 6) * 15.0 + 3.0, 7.3)).collect());
    s.engrave = vec![pts(&[(1.0, 79.0), (30.0, 79.0)])];
    let p = GcodeParams::for_material("film");
    let a = laser_gcode(&s, &p);
    let b = laser_gcode(&s, &p);
    assert_eq!(a, b);
    assert_safe(&a);
}

#[test]
fn too_many_lines_are_refused() {
    // 110 000 points × 20 passes > 2 000 000 lines.
    let ring: Vec<Point> =
        (0..110_000).map(|i| Point::new(50.0, 50.0) + vectorcraft_geom::kurbo::Vec2::from_angle(f64::from(i) * 1e-4) * 40.0).collect();
    let s = sheet(100.0, 100.0, vec![ring]);
    let mut p = GcodeParams::default();
    p.cut.passes = 20;
    let g = laser_gcode(&s, &p);
    assert!(g.text.is_empty() && g.lines == 0);
    assert!(g.warnings.iter().any(|w| w.contains(&MAX_GCODE_LINES.to_string())), "{:?}", g.warnings);
    p.cut.passes = 1;
    let g = laser_gcode(&s, &p);
    assert_safe(&g);
    assert!(g.lines > 110_000 && g.lines < MAX_GCODE_LINES);
}

#[test]
fn params_clamp_and_read_json_leniently() {
    let p = GcodeParams::default();
    assert_eq!((p.s_max, p.cut.speed_mm_min, p.cut.power_pct, p.cut.passes), (1000.0, 300.0, 100.0, 2));
    assert_eq!((p.score.speed_mm_min, p.score.power_pct, p.score.passes), (1500.0, 30.0, 1));
    assert_eq!((p.engrave.speed_mm_min, p.engrave.power_pct, p.engrave.passes), (2000.0, 20.0, 1));
    assert_eq!((p.origin_mm, p.air), (Point::ZERO, false));
    // Junk struct values clamp or fall back.
    let junk = GcodeParams {
        s_max: f64::NAN,
        cut: OpSettings { enabled: true, speed_mm_min: -5.0, power_pct: 400.0, passes: 0 },
        score: OpSettings { enabled: false, speed_mm_min: f64::INFINITY, power_pct: f64::NAN, passes: 999 },
        engrave: p.engrave,
        origin_mm: Point::new(1e9, f64::NEG_INFINITY),
        air: true,
        header: Vec::new(),
        head: p.head,
        knife: p.knife,
    }
    .clamped();
    assert_eq!(junk.s_max, 1000.0);
    assert_eq!((junk.cut.speed_mm_min, junk.cut.power_pct, junk.cut.passes), (1.0, 100.0, 1));
    assert_eq!((junk.score.enabled, junk.score.speed_mm_min, junk.score.power_pct, junk.score.passes), (false, 1500.0, 30.0, 20));
    assert_eq!(junk.origin_mm, Point::new(5000.0, 0.0));
    // JSON: partial op objects overlay field by field; wrong types keep the value.
    let v = json!({
        "sMax": 255, "originXMm": 12.5, "originYMm": "x", "air": "yes",
        "cut": {"powerPct": 80, "passes": 2.6, "speedMmMin": null},
        "score": {"enabled": false},
        "engrave": "loud",
        "header": ["a", 3, "b"],
    });
    let q = GcodeParams::from_json(&v);
    assert_eq!((q.s_max, q.origin_mm, q.air), (255.0, Point::new(12.5, 0.0), false));
    assert_eq!((q.cut.power_pct, q.cut.passes, q.cut.speed_mm_min, q.cut.enabled), (80.0, 3, 300.0, true));
    assert!(!q.score.enabled && q.score.speed_mm_min == 1500.0);
    assert_eq!(q.engrave, p.engrave);
    assert_eq!(q.header, vec!["a".to_string(), "b".to_string()]);
    let huge = GcodeParams::from_json(&json!({"sMax": 1e300, "cut": {"speedMmMin": -1e300, "passes": -4}, "originXMm": -1e12}));
    assert_eq!((huge.s_max, huge.cut.speed_mm_min, huge.cut.passes, huge.origin_mm.x), (100_000.0, 1.0, 1, -5000.0));
    for junk in [json!(null), json!(7), json!("x"), json!([1, 2]), json!({"cut": [1]})] {
        assert_eq!(GcodeParams::from_json(&junk), GcodeParams::default());
    }
    // Round trip (the header isn't remembered).
    let back = GcodeParams::from_json(&q.to_json());
    assert_eq!(back, GcodeParams { header: Vec::new(), ..q.clone() });
    assert!(q.to_json().get("header").is_none());
    assert_eq!(q.to_json()["cut"], json!({"enabled": true, "speedMmMin": 300.0, "powerPct": 80.0, "passes": 3}));
    // apply_json layers on top of what's there.
    let mut film = GcodeParams::for_material("film");
    film.apply_json(&json!({"cut": {"passes": 5}}));
    assert_eq!((film.cut.passes, film.cut.speed_mm_min), (5, GcodeParams::for_material("film").cut.speed_mm_min));
}

#[test]
fn materials_have_their_own_starting_settings() {
    let paper = GcodeParams::for_material("paper");
    assert_eq!(paper, GcodeParams::default());
    assert_eq!(GcodeParams::for_material("custom"), paper);
    assert_eq!(GcodeParams::for_material("unknown"), paper);
    let film = GcodeParams::for_material("Film");
    let card = GcodeParams::for_material("card");
    assert_ne!(film, paper);
    assert_ne!(card, paper);
    assert_ne!(card, film);
    for p in [&paper, &film, &card] {
        assert_eq!(p.clamped(), *p, "presets are in range");
    }
}

/// The P12 sample stencil: a 200 × 120 mm design with a 15 mm margin (sheet 230 × 150 mm), kerf
/// 0.1 (page 230.1 × 150.1 mm), its G-code with the origin at 0,0.
#[test]
fn the_sample_stencil_spans_its_sheet() {
    let mut cut = BezPath::new();
    for (x, y, r) in [(60.0, 50.0, 18.0), (115.0, 75.0, 25.0), (175.0, 55.0, 14.0)] {
        cut.extend(Circle::new((x, y), r).path_elements(0.05));
    }
    cut.extend(Rect::new(40.0, 100.0, 190.0, 108.0).path_elements(0.05));
    let input = LaserInput {
        cut,
        sheet: Rect::new(0.0, 0.0, 230.0, 150.0),
        mm_per_unit: 1.0,
        hoshi_d_mm: 1.5,
        label_lines: vec!["Sample".into(), "Sheet 230 × 150 mm".into()],
        ..LaserInput::default()
    };
    let job = laser_job(&input, &LaserParams::default());
    assert_eq!(job.sheets.len(), 1, "{:?}", job.warnings);
    let s = &job.sheets[0];
    assert!((s.w_mm - 230.1).abs() < 1e-9 && (s.h_mm - 150.1).abs() < 1e-9);
    assert!(!s.engrave.is_empty(), "labelled");
    let g = laser_gcode(s, &GcodeParams::default());
    assert_safe(&g);
    let b = g.bounds_mm;
    assert!(b.x0.abs() < 1e-3 && b.y0.abs() < 1e-3, "{b:?}");
    assert!((b.x1 - 230.1).abs() < 1e-3 && (b.y1 - 150.1).abs() < 1e-3, "{b:?}");
    for (_, x, y) in moves(&g) {
        assert!((-1e-3..=230.101).contains(&x) && (-1e-3..=150.101).contains(&y), "{x} {y}");
    }
    // The outline is cut last: the final burn ends on the sheet's edge.
    let m = moves(&g);
    let last_burn = m.iter().rev().find(|k| k.0 == 1).unwrap();
    assert!(last_burn.1.abs() < 1e-3 || last_burn.2.abs() < 1e-3 || (last_burn.1 - 230.1).abs() < 1e-3 || (last_burn.2 - 150.1).abs() < 1e-3);
    assert_eq!(s_values(&g), vec![200.0, 1000.0], "label engraved, then cut");
    // The same stencil 30 mm in from the bed's corner moves with it.
    let moved = laser_gcode(s, &GcodeParams { origin_mm: Point::new(30.0, 40.0), ..GcodeParams::default() });
    let mb = moved.bounds_mm;
    assert!((mb.x0 - 30.0).abs() < 1e-3 && (mb.y1 - 190.1).abs() < 1e-3, "{mb:?}");
}

/// The P12 test card's G-code: the labels engraved, then the cut; within its page.
#[test]
fn the_test_card_runs_as_gcode() {
    let card = test_card(&StencilMaterial::Paper.limits().unwrap(), "Paper", 0.1);
    let g = laser_gcode(&card, &GcodeParams::for_material("paper"));
    assert_safe(&g);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    assert_eq!(s_values(&g), vec![200.0, 1000.0]);
    let b = g.bounds_mm;
    assert!(b.x0.abs() < 1e-3 && b.y0.abs() < 1e-3 && (b.x1 - 120.1).abs() < 1e-3 && (b.y1 - 80.1).abs() < 1e-3, "{b:?}");
    assert!(g.text.contains("; sheet: test card"));
    // Every cut contour is closed in the G-code: as many cut G0s as contours (2 passes in place).
    let c = code(&g);
    let cut_start = c.iter().rposition(|l| l.starts_with("M4")).unwrap();
    let travels = c[cut_start..].iter().filter(|l| l.starts_with("G0")).count();
    assert_eq!(travels, card.cut.len() + 1, "one travel per contour, plus the park");
}

/// Random sheets and junk params: the file is always either refused with a reason or safe.
#[test]
fn random_sheets_always_give_safe_gcode() {
    let mut rng = Rng::new(0x6c61_7365);
    let junk = [f64::NAN, f64::INFINITY, -1e300, 1e30, -0.0];
    for round in 0..300 {
        let w = rng.range(1.0, 800.0);
        let h = rng.range(1.0, 800.0);
        let point = |rng: &mut Rng| {
            if rng.next().is_multiple_of(40) {
                let j = junk[(rng.next() % junk.len() as u64) as usize];
                if rng.next().is_multiple_of(2) { Point::new(j, rng.range(0.0, h)) } else { Point::new(rng.range(0.0, w), j) }
            } else {
                Point::new(rng.range(-5.0, w + 5.0), rng.range(-5.0, h + 5.0))
            }
        };
        let paths =
            |rng: &mut Rng, n: u64| -> Vec<Vec<Point>> { (0..rng.next() % n).map(|_| (0..rng.next() % 12).map(|_| point(rng)).collect()).collect() };
        let mut s = sheet(w, h, paths(&mut rng, 8));
        s.score = paths(&mut rng, 3);
        s.engrave = paths(&mut rng, 5);
        let any = |rng: &mut Rng, lo: f64, hi: f64| -> serde_json::Value {
            match rng.next() % 6 {
                0 => json!("junk"),
                1 => json!(null),
                2 => json!(-1e300),
                3 => json!(1e300),
                _ => json!(rng.range(lo, hi)),
            }
        };
        let op = |rng: &mut Rng| json!({"enabled": !rng.next().is_multiple_of(4), "speedMmMin": any(rng, 0.0, 30_000.0), "powerPct": any(rng, -10.0, 120.0), "passes": any(rng, 0.0, 30.0)});
        let v = json!({
            "sMax": any(&mut rng, 0.0, 200_000.0),
            "originXMm": any(&mut rng, -6000.0, 6000.0),
            "originYMm": any(&mut rng, -6000.0, 6000.0),
            "air": rng.next().is_multiple_of(2),
            "cut": op(&mut rng),
            "score": op(&mut rng),
            "engrave": op(&mut rng),
        });
        let p = GcodeParams::from_json(&v);
        let g = laser_gcode(&s, &p);
        if g.text.is_empty() {
            assert!(!g.warnings.is_empty() && g.lines == 0, "round {round}: refused without a reason");
            continue;
        }
        assert_safe(&g);
        assert!(g.burn_mm.is_finite() && g.travel_mm.is_finite() && g.burn_sec.is_finite(), "round {round}");
        assert!(estimate_sec(&g, 3000.0).is_finite());
        let b = g.bounds_mm;
        let (ox, oy) = (p.origin_mm.x, p.origin_mm.y);
        assert!(b.x0 >= ox - 5.01 && b.x1 <= ox + w + 5.01 && b.y0 >= oy - 5.01 && b.y1 <= oy + h + 5.01, "round {round}: {b:?}");
        assert_eq!(g, laser_gcode(&s, &p), "round {round}: deterministic");
    }
}

/// Timing (`cargo test --release -p vectorcraft-stencil -- --ignored --nocapture gcode_timing`): a
/// sheet of 2000 jagged contours, 100 segments each (200k segments).
#[test]
#[ignore]
fn gcode_timing() {
    let mut cut = Vec::new();
    for i in 0..2000 {
        let (cx, cy) = (f64::from(i % 50) * 11.0 + 6.0, f64::from(i / 50) * 11.0 + 6.0);
        cut.push(
            (0..100)
                .map(|k| {
                    let a = f64::from(k) / 100.0 * std::f64::consts::TAU;
                    let r = 4.0 + 0.6 * (f64::from(k) * 1.7).sin();
                    Point::new(cx + r * a.cos(), cy + r * a.sin())
                })
                .collect(),
        );
    }
    cut.push(square(0.0, 0.0, 560.0));
    let s = sheet(560.0, 560.0, cut);
    let p = GcodeParams::default();
    let t = Instant::now();
    let g = laser_gcode(&s, &p);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_safe(&g);
    println!(
        "gcode: {} contours, {} lines, {} bytes, {ms:.1} ms; burn {:.0} mm, travel {:.0} mm, estimate {:.0} s",
        s.cut.len(),
        g.lines,
        g.text.len(),
        g.burn_mm,
        g.travel_mm,
        estimate_sec(&g, 3000.0)
    );
}
