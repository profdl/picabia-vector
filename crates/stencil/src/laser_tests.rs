//! Laser export: kerf, clean geometry, cut order, bed split, label, SVG, params.

use serde_json::json;
use vectorcraft_geom::kurbo::{BezPath, Circle, Point, Rect, Shape};

use crate::laser::{bounds, clean_contour, dedup};
use crate::laser_svg::num;
use crate::{LaserInput, LaserJob, LaserParams, LaserSheet, laser_job, laser_svg};

fn square(x: f64, y: f64, s: f64) -> BezPath {
    Rect::new(x, y, x + s, y + s).to_path(0.01)
}

/// A 1 mm-per-unit job of `cut` on a `w` × `h` sheet.
fn input(cut: BezPath, w: f64, h: f64) -> LaserInput {
    LaserInput { cut, sheet: Rect::new(0.0, 0.0, w, h), mm_per_unit: 1.0, hoshi_d_mm: 1.5, ..LaserInput::default() }
}

fn no_bed() -> LaserParams {
    LaserParams { bed_w_mm: 0.0, bed_h_mm: 0.0, ..LaserParams::default() }
}

fn only(job: &LaserJob) -> &LaserSheet {
    assert_eq!(job.sheets.len(), 1, "{:?}", job.warnings);
    &job.sheets[0]
}

/// The contour whose bounds' centre is nearest `c`.
fn contour_at(sheet: &LaserSheet, c: Point) -> Rect {
    sheet.cut.iter().map(|k| bounds(k)).min_by(|a, b| (a.center() - c).hypot().total_cmp(&(b.center() - c).hypot())).unwrap()
}

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn kerf_shrinks_cut_outs_and_grows_the_outline() {
    let job = laser_job(&input(square(40.0, 40.0, 10.0), 100.0, 100.0), &no_bed());
    let s = only(&job);
    assert!(job.warnings.is_empty(), "{:?}", job.warnings);
    assert!(near(s.w_mm, 100.1, 1e-9) && near(s.h_mm, 100.1, 1e-9));
    assert_eq!(s.cut.len(), 2, "the cut-out and the outline");
    // Sheet-local: the outline's corner is the origin, so the design moved by kerf / 2.
    let hole = contour_at(s, Point::new(45.05, 45.05));
    assert!(near(hole.width(), 9.9, 2e-3) && near(hole.height(), 9.9, 2e-3), "{hole:?}");
    assert!(near(hole.x0, 40.1, 2e-3) && near(hole.y0, 40.1, 2e-3), "{hole:?}");
    let outline = bounds(s.cut.last().unwrap());
    assert!(near(outline.x0, 0.0, 1e-6) && near(outline.y0, 0.0, 1e-6), "{outline:?}");
    assert!(near(outline.width(), 100.1, 2e-3) && near(outline.height(), 100.1, 2e-3), "the outline 0.05 out: {outline:?}");
    // Kerf 0 leaves the geometry alone.
    let raw = laser_job(&input(square(40.0, 40.0, 10.0), 100.0, 100.0), &LaserParams { kerf_mm: 0.0, ..no_bed() });
    let s = only(&raw);
    assert!(near(s.w_mm, 100.0, 1e-9));
    let hole = contour_at(s, Point::new(45.0, 45.0));
    assert!(near(hole.width(), 10.0, 1e-6) && near(hole.x0, 40.0, 1e-6), "{hole:?}");
    // Layer units: 2 mm per unit doubles everything, the kerf stays in mm.
    let half = LaserInput { cut: square(20.0, 20.0, 5.0), sheet: Rect::new(0.0, 0.0, 50.0, 50.0), mm_per_unit: 2.0, ..LaserInput::default() };
    let s = laser_job(&half, &no_bed());
    let hole = contour_at(only(&s), Point::new(45.05, 45.05));
    assert!(near(hole.width(), 9.9, 2e-3), "{hole:?}");
}

#[test]
fn cut_outs_narrower_than_the_kerf_vanish_with_a_warning() {
    let mut cut = Rect::new(10.0, 10.0, 10.06, 30.0).to_path(0.01);
    cut.extend(Rect::new(20.0, 10.0, 20.08, 30.0).to_path(0.01).iter());
    cut.extend(square(40.0, 10.0, 5.0).iter());
    let job = laser_job(&input(cut, 60.0, 40.0), &LaserParams { label: false, ..no_bed() });
    assert_eq!(only(&job).cut.len(), 2, "the square and the outline");
    assert!(job.warnings.iter().any(|w| w == "2 cut-outs are narrower than the kerf and were left out"), "{:?}", job.warnings);
}

#[test]
fn contours_are_cleaned() {
    let ring = vec![
        Point::new(0.0, 0.0),
        Point::new(0.0, 0.0),
        Point::new(5.0, 0.001),
        Point::new(10.0, 0.0),
        Point::new(10.0, 0.005),
        Point::new(10.0, 10.0),
        Point::new(5.0, 10.0),
        Point::new(0.0, 10.0),
        Point::new(0.004, 0.0),
    ];
    let c = clean_contour(&ring).unwrap();
    assert_eq!(c.len(), 4, "duplicates, a 0.005 mm segment, collinear points within 0.002 mm: {c:?}");
    assert!(clean_contour(&[Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)]).is_none(), "no area");
    assert!(clean_contour(&[Point::new(0.0, 0.0), Point::new(0.005, 0.0), Point::new(0.0, 0.005)]).is_none(), "tiny");
    assert!(clean_contour(&[Point::new(0.0, 0.0), Point::new(f64::NAN, 1.0), Point::new(1.0, 1.0)]).is_none());
    // The same contour reversed and from another start, and one nearly equal, are duplicates.
    let a = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    let mut b: Vec<Point> = a.iter().rev().copied().collect();
    b.rotate_left(2);
    let c2: Vec<Point> = a.iter().map(|p| Point::new(p.x + 0.0005, p.y)).collect();
    let other = vec![Point::new(20.0, 0.0), Point::new(30.0, 0.0), Point::new(30.0, 10.0), Point::new(20.0, 10.0)];
    let off: Vec<Point> = a.iter().map(|p| Point::new(p.x + 0.01, p.y)).collect();
    let out = dedup(vec![a.clone(), b, c2, other.clone(), off]);
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[0], a);
    assert_eq!(out[1], other);
}

/// Even-odd nested squares: a cut-out (20–80), an island in it (30–70), a cut-out in the island
/// (40–60); plus three small cut-outs in a row.
fn nested() -> BezPath {
    let mut cut = square(20.0, 20.0, 60.0);
    cut.extend(square(30.0, 30.0, 40.0).iter());
    cut.extend(square(40.0, 40.0, 20.0).iter());
    cut
}

#[test]
fn cut_order_is_deepest_first_and_the_outline_last() {
    let mut cut = nested();
    for x in [85.0, 3.0, 45.0] {
        cut.extend(square(x, 88.0, 4.0).iter());
    }
    let job = laser_job(&input(cut, 100.0, 100.0), &LaserParams { kerf_mm: 0.0, label: false, ..no_bed() });
    let s = only(&job);
    let b: Vec<Rect> = s.cut.iter().map(|c| bounds(c)).collect();
    assert_eq!(b.len(), 7, "{b:?}");
    let w = |r: &Rect| r.width().round() as i64;
    assert_eq!(w(&b[0]), 20, "the cut-out in the island first");
    assert_eq!(w(&b[1]), 40, "then the island's outline");
    // Depth 1: the big cut-out and the three small ones, nearest-neighbour from the island.
    let depth1: Vec<i64> = b[2..6].iter().map(w).collect();
    assert_eq!(depth1.iter().filter(|&&x| x == 4).count(), 3);
    let small_x: Vec<i64> = b[2..6].iter().filter(|r| w(r) == 4).map(|r| r.x0.round() as i64).collect();
    assert!(small_x == vec![45, 3, 85] || small_x == vec![3, 45, 85] || small_x == vec![85, 45, 3], "a path, not a jump back and forth: {small_x:?}");
    assert_eq!(w(&b[6]), 100, "the outline last");
    // Each contour starts at its vertex nearest the previous start.
    assert_eq!(s.loose_pieces, 1, "the island has no ties");
    assert!(job.warnings.iter().any(|w| w == "1 piece would fall out of the sheet; add ties"), "{:?}", job.warnings);
    // Deterministic.
    assert_eq!(laser_job(&input(nested(), 100.0, 100.0), &no_bed()), laser_job(&input(nested(), 100.0, 100.0), &no_bed()));
}

#[test]
fn greedy_order_follows_the_head() {
    let mut cut = BezPath::new();
    for x in [80.0, 10.0, 45.0] {
        cut.extend(square(x, 10.0, 5.0).iter());
    }
    let job = laser_job(&input(cut, 100.0, 30.0), &LaserParams { kerf_mm: 0.0, label: false, ..no_bed() });
    let xs: Vec<i64> = only(&job).cut.iter().map(|c| c[0].x.round() as i64).collect();
    assert_eq!(xs, vec![10, 45, 80, 100], "from the top-left, each started at its corner nearest the head (the outline too)");
}

/// A 1200 × 300 job: a ring cut-out 10 mm wide around an island 110 mm across held by one tie on
/// its right, centred where the first split must fall.
fn long_job() -> LaserInput {
    let c = Point::new(400.0, 150.0);
    let mut ring = Circle::new(c, 120.0).to_path(0.01);
    ring.extend(Circle::new(c, 110.0).to_path(0.01).iter());
    let ring = crate::ops::simplify_with_rule(&ring, true).unwrap();
    let tie = Rect::new(505.0, 147.0, 525.0, 153.0).to_path(0.01);
    let cut = crate::ops::boolean_step(&ring, &tie, crate::ops::BoolOp::Subtract).unwrap();
    LaserInput { label_lines: vec!["SPLIT TEST".into()], ..input(cut, 1200.0, 300.0) }
}

/// Small round contours (holes) on a sheet: their centres.
fn holes(s: &LaserSheet, d: f64) -> Vec<Point> {
    s.cut.iter().map(|c| bounds(c)).filter(|b| (b.width() - d).abs() < 0.3 && (b.height() - d).abs() < 0.3).map(|b| b.center()).collect()
}

#[test]
fn a_long_job_splits_into_overlapping_sheets() {
    let p = LaserParams::default();
    let job = laser_job(&long_job(), &p);
    assert_eq!(job.sheets.len(), 3, "{:?}", job.warnings);
    let names: Vec<&str> = job.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["1 of 3", "2 of 3", "3 of 3"]);
    let total_w: f64 = job.sheets.iter().map(|s| s.w_mm - p.kerf_mm).sum();
    assert!(near(total_w, 1200.0 + 2.0 * p.overlap_mm, 1e-6), "the sheets overlap by 10 mm: {total_w}");
    for s in &job.sheets {
        let fits = (s.w_mm <= 600.0 && s.h_mm <= 400.0) || (s.w_mm <= 400.0 && s.h_mm <= 600.0);
        assert!(fits, "{} × {}", s.w_mm, s.h_mm);
        assert!(near(s.h_mm, 300.1, 1e-9));
        let outline = bounds(s.cut.last().unwrap());
        assert!(near(outline.width(), s.w_mm, 2e-3), "the outline last, page-sized: {outline:?}");
    }
    // Registration holes: two per shared edge, at the same place on both sheets (measured from
    // the shared band's middle: 5 mm plus half the kerf in from each sheet's edge).
    let (s1, s2, s3) = (&job.sheets[0], &job.sheets[1], &job.sheets[2]);
    let d = 1.5 - p.kerf_mm;
    let (h1, h2, h3) = (holes(s1, d), holes(s2, d), holes(s3, d));
    assert_eq!((h1.len(), h2.len(), h3.len()), (2, 4, 2), "{h1:?} {h2:?} {h3:?}");
    for (a, b) in h1.iter().zip(h2.iter().filter(|q| q.x < s2.w_mm / 2.0)) {
        assert!(near(s1.w_mm - a.x, 5.05, 1e-3) && near(b.x, 5.05, 1e-3), "{a:?} {b:?}");
        assert!(near(a.y, b.y, 1e-6), "{a:?} {b:?}");
    }
    // Score lines where the neighbour's edge lands.
    assert_eq!((s1.score.len(), s2.score.len(), s3.score.len()), (1, 2, 1));
    assert!(near(s1.score[0][0].x, s1.w_mm - 10.05, 1e-6), "{:?}", s1.score);
    // The island's left part on sheet 1 lost its tie.
    assert!(s1.loose_pieces >= 1, "{}", s1.loose_pieces);
    assert_eq!(s2.loose_pieces, 0);
    assert!(
        job.warnings.iter().any(|w| w.starts_with("sheet 1 of 3: ") && w.ends_with("would fall out; move the split or add ties")),
        "{:?}",
        job.warnings
    );
    // Labels: name, sheet number, this side up, on every sheet.
    assert!(job.sheets.iter().all(|s| !s.engrave.is_empty()));
    // Unsplit, the island holds.
    let whole = laser_job(&long_job(), &no_bed());
    assert_eq!(only(&whole).loose_pieces, 0, "{:?}", whole.warnings);
}

#[test]
fn split_lines_follow_an_axis_aligned_repeat_cell() {
    let cell = [Point::new(0.0, 0.0), Point::new(150.0, 0.0), Point::new(150.0, 300.0), Point::new(0.0, 300.0)];
    let inp = LaserInput { repeat_cell: Some(cell), ..input(BezPath::new(), 1200.0, 300.0) };
    let job = laser_job(&inp, &LaserParams::default());
    let w: Vec<f64> = job.sheets.iter().map(|s| (s.w_mm * 10.0).round() / 10.0).collect();
    assert_eq!(w, vec![455.1, 310.1, 455.1], "splits at 450 and 750: {:?}", job.warnings);
    // A turned cell is ignored (even spacing: 400, 800).
    let turned = [Point::new(0.0, 0.0), Point::new(150.0, 20.0), Point::new(150.0, 320.0), Point::new(0.0, 300.0)];
    let job = laser_job(&LaserInput { repeat_cell: Some(turned), ..inp }, &LaserParams::default());
    let w: Vec<f64> = job.sheets.iter().map(|s| (s.w_mm * 10.0).round() / 10.0).collect();
    assert_eq!(w, vec![405.1, 410.1, 405.1]);
}

#[test]
fn splits_avoid_the_cut() {
    // Cut-outs everywhere but a clear band at x 330–350: the first split goes there.
    let mut cut = BezPath::new();
    let mut x = 5.0;
    while x < 1190.0 {
        if !(320.0..355.0).contains(&x) {
            cut.extend(Rect::new(x, 20.0, x + 3.0, 280.0).to_path(0.01).iter());
        }
        x += 5.0;
    }
    let job = laser_job(&input(cut, 1200.0, 300.0), &LaserParams { label: false, ..LaserParams::default() });
    assert_eq!(job.sheets.len(), 3);
    let first = job.sheets[0].w_mm - 0.1 - 5.0;
    assert!((330.0..=350.0).contains(&first), "split at {first}");
}

#[test]
fn bed_fit_rotation_and_the_sheet_cap() {
    let job = laser_job(&input(square(10.0, 10.0, 5.0), 300.0, 550.0), &LaserParams::default());
    assert_eq!(job.sheets.len(), 1);
    assert!(job.warnings.iter().any(|w| w == "the sheet fits the bed turned 90°"), "{:?}", job.warnings);
    let job = laser_job(&input(BezPath::new(), 5000.0, 5000.0), &LaserParams { bed_w_mm: 10.0, bed_h_mm: 10.0, ..LaserParams::default() });
    assert!(job.sheets.is_empty());
    assert!(job.warnings.iter().any(|w| w.contains("at most 64 are made")), "{:?}", job.warnings);
    // 2 × 2 grid: holes on all four shared edges.
    let job = laser_job(&input(BezPath::new(), 1000.0, 700.0), &LaserParams::default());
    assert_eq!(job.sheets.len(), 4, "{:?}", job.warnings);
    assert!(job.sheets.iter().all(|s| holes(s, 1.4).len() == 4), "{:?}", job.sheets.iter().map(|s| holes(s, 1.4)).collect::<Vec<_>>());
}

#[test]
fn junk_input_is_refused_or_survived() {
    let p = LaserParams::default();
    for mpu in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let job = laser_job(&LaserInput { mm_per_unit: mpu, ..input(square(1.0, 1.0, 1.0), 10.0, 10.0) }, &p);
        assert!(job.sheets.is_empty() && !job.warnings.is_empty(), "{mpu}");
    }
    let job = laser_job(&LaserInput { sheet: Rect::new(0.0, 0.0, f64::INFINITY, 1.0), ..input(BezPath::new(), 1.0, 1.0) }, &p);
    assert!(job.sheets.is_empty());
    let job = laser_job(&input(BezPath::new(), 0.0, 10.0), &p);
    assert!(job.sheets.is_empty());
    let job = laser_job(&input(BezPath::new(), 20_000.0, 10.0), &p);
    assert!(job.sheets.is_empty());
    let mut bad = BezPath::new();
    bad.move_to((1.0, 1.0));
    bad.line_to((f64::NAN, 2.0));
    bad.line_to((3.0, 1.0));
    bad.close_path();
    assert!(laser_job(&input(bad, 10.0, 10.0), &p).sheets.is_empty());
    // Nothing cut: the outline alone.
    let job = laser_job(&input(BezPath::new(), 50.0, 50.0), &p);
    assert_eq!(only(&job).cut.len(), 1);
    // Cut past the sheet is clipped to it; an open subpath counts closed.
    let mut open = BezPath::new();
    open.move_to((40.0, 40.0));
    open.line_to((70.0, 40.0));
    open.line_to((70.0, 70.0));
    let job = laser_job(&input(open, 50.0, 50.0), &p);
    let s = only(&job);
    assert!(s.cut.iter().all(|c| c.iter().all(|q| q.x >= -1e-6 && q.x <= s.w_mm + 1e-6 && q.y >= -1e-6 && q.y <= s.h_mm + 1e-6)));
    // Junk params are clamped.
    let wild = LaserParams {
        kerf_mm: f64::NAN,
        bed_w_mm: -5.0,
        bed_h_mm: 1e9,
        overlap_mm: 1e9,
        cut: "red".into(),
        engrave: String::new(),
        score: "#12".into(),
        label: true,
    };
    let job = laser_job(&input(square(10.0, 10.0, 5.0), 50.0, 50.0), &wild);
    assert_eq!(job.sheets.len(), 1, "{:?}", job.warnings);
}

#[test]
fn labels_go_in_the_bottom_margin_clear_of_the_cut() {
    let mut cut = square(15.0, 15.0, 70.0);
    // A registration hole in the bottom-left margin corner.
    cut.extend(Circle::new((7.5, 92.5), 0.75).to_path(0.01).iter());
    let inp = LaserInput { label_lines: vec!["Swoosh".into(), "Sheet 100 × 100 mm".into(), "paper".into()], ..input(cut, 100.0, 100.0) };
    let job = laser_job(&inp, &LaserParams::default());
    let s = only(&job);
    assert!(job.warnings.is_empty(), "{:?}", job.warnings);
    assert!(!s.engrave.is_empty());
    let all = s.engrave.iter().flatten().fold(Rect::from_points(s.engrave[0][0], s.engrave[0][0]), |b, &p| b.union_pt(p));
    assert!(all.y0 > 85.0 && all.y1 < 100.0 && all.x0 >= 1.5, "bottom margin from the left: {all:?}");
    // Away from the frayable edges: 6 mm beyond the 1.5 mm padding fits in the 15 mm margin.
    assert!(all.x0 >= 7.5 && all.y1 <= 92.6, "inset from the edges: {all:?}");
    // Off: nothing engraved.
    let job = laser_job(&inp, &LaserParams { label: false, ..LaserParams::default() });
    assert!(only(&job).engrave.is_empty());
    // No margin: no room, a warning.
    let full = LaserInput { label_lines: vec!["X".into()], ..input(square(0.5, 0.5, 99.0), 100.0, 100.0) };
    let job = laser_job(&full, &LaserParams::default());
    assert!(only(&job).engrave.is_empty());
    assert!(job.warnings.iter().any(|w| w == "no room for the label clear of the cut; it was left out"), "{:?}", job.warnings);
    // Characters the font lacks: a warning.
    let odd = LaserInput { label_lines: vec!["Ünïcode ✓".into()], ..input(BezPath::new(), 100.0, 100.0) };
    let job = laser_job(&odd, &LaserParams::default());
    assert!(job.warnings.iter().any(|w| w.contains("engrave as boxes")), "{:?}", job.warnings);
}

#[test]
fn svg_has_the_size_layers_and_hairlines() {
    let job = laser_job(&input(nested(), 100.0, 100.0), &no_bed());
    let s = only(&job);
    let svg = laser_svg(s, &LaserParams::default());
    assert!(svg.contains(r#"width="100.1mm" height="100.1mm" viewBox="0 0 100.1 100.1""#), "{}", &svg[..300]);
    assert!(svg.contains(r#"xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape""#));
    assert!(svg.contains("<title>Picabia laser: 1 of 1</title>"));
    let e = svg
        .find(r##"<g id="Engrave" inkscape:groupmode="layer" inkscape:label="Engrave" fill="none" stroke="#000000" stroke-width="0.01">"##)
        .unwrap();
    let c = svg.find(r##"<g id="Cut" inkscape:groupmode="layer" inkscape:label="Cut" fill="none" stroke="#ff0000" stroke-width="0.01">"##).unwrap();
    assert!(e < c, "engrave before cut");
    assert!(!svg.contains(r#"id="Score""#), "empty groups are left out");
    assert_eq!(svg.matches("Z\"/>").count(), s.cut.len(), "every cut contour closed");
    assert!(svg[e..c].matches("<path").count() == s.engrave.len() && !svg[e..c].contains('Z'), "polylines stay open");
    for d in svg.split(" d=\"").skip(1).map(|t| t.split('"').next().unwrap()) {
        assert!(!d.contains('e') && !d.contains("NaN") && !d.contains("inf"), "{d}");
        for n in d.split(|c: char| c.is_ascii_alphabetic() || c == ' ').filter(|t| !t.is_empty()) {
            assert!(n.split('.').nth(1).is_none_or(|f| f.len() <= 3), "{n}");
        }
    }
    let split = laser_job(&long_job(), &split_params());
    let svg = laser_svg(&split.sheets[1], &split_params());
    let (e, sc, c) = (svg.find(r#"id="Engrave""#).unwrap(), svg.find(r#"id="Score""#).unwrap(), svg.find(r#"id="Cut""#).unwrap());
    assert!(e < sc && sc < c, "engrave, score, cut");
    assert!(svg.contains(r##"stroke="#00ff00""##));
    // Titles are escaped.
    let odd = LaserSheet { name: "<a & b>".into(), w_mm: 1.0, h_mm: 1.0, ..LaserSheet::default() };
    assert!(laser_svg(&odd, &LaserParams::default()).contains("<title>Picabia laser: &lt;a &amp; b&gt;</title>"));
}

fn split_params() -> LaserParams {
    LaserParams { score: "#00ff00".into(), ..LaserParams::default() }
}

#[test]
fn numbers_have_three_decimals_and_no_exponent() {
    assert_eq!(num(1e-7), "0");
    assert_eq!(num(-0.0001), "0");
    assert_eq!(num(1_234_567.0), "1234567");
    assert_eq!(num(12.5), "12.5");
    assert_eq!(num(0.1 + 0.2), "0.3");
    assert_eq!(num(-1.23456), "-1.235");
    assert_eq!(num(f64::NAN), "0");
    assert_eq!(num(1e21), "1000000000000000000000");
}

#[test]
fn params_from_json_are_lenient_and_clamped() {
    assert_eq!(LaserParams::from_json(&json!(null)), LaserParams::default());
    let p = LaserParams::from_json(&json!({
        "kerfMm": 9, "bedWidthMm": 5, "bedHeightMm": 0, "overlapMm": -3,
        "cutColour": "#00FF00", "engraveColour": "nope", "scoreColour": 7, "label": false, "extra": 1
    }));
    assert_eq!(p.kerf_mm, 2.0);
    assert_eq!(p.bed_w_mm, 10.0);
    assert_eq!(p.bed_h_mm, 0.0, "0 = no limit");
    assert_eq!(p.overlap_mm, 0.0);
    assert_eq!(p.cut, "#00ff00");
    assert_eq!(p.engrave, "#000000", "bad hex keeps the default");
    assert_eq!(p.score, "#0000ff");
    assert!(!p.label);
    assert_eq!(LaserParams::from_json(&p.to_json()), p, "round trip");
    let d = LaserParams::default().to_json();
    assert_eq!(
        d,
        json!({"kerfMm": 0.1, "bedWidthMm": 600.0, "bedHeightMm": 400.0, "overlapMm": 10.0, "cutColour": "#ff0000", "engraveColour": "#000000", "scoreColour": "#0000ff", "label": true})
    );
    // Layering: only the fields given change.
    let mut q = p.clone();
    q.apply_json(&json!({"kerfMm": 0.15, "label": true}));
    assert_eq!((q.kerf_mm, q.label, q.cut.as_str(), q.bed_w_mm), (0.15, true, "#00ff00", 10.0));
    q.apply_json(&json!([1, 2]));
    assert_eq!(q.kerf_mm, 0.15);
}
