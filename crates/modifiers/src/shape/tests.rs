// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, shapes};

use super::pieces::{Measured, Piece, from_pieces, pieces};
use super::{GeometryOp, MAX_GEOMETRY_ANCHORS, RowAt, Shaper, resample, tidy, trim};
use crate::model::{CloseMethod, Resample, ResampleMode, SortBy, SubdivideMethod, Tidy, Trim};
use crate::{Output, evaluate, parse};

fn pts(sp: &SubPath) -> Vec<Point> {
    sp.anchors.iter().map(|a| a.p).collect()
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

fn xs(sp: &[Point]) -> Vec<f64> {
    sp.iter().map(|p| (p.x * 1e6).round() / 1e6).collect()
}

// ---- Trim: tlprocedural's `extractSubPath` tests (pathLineUtils.test.ts), same expectations ----

/// Horizontal line 0 → 10 → 20 → 30 → 40 at y = 0, total length 40.
fn line() -> SubPath {
    SubPath::polyline(&[0.0, 10.0, 20.0, 30.0, 40.0].map(|x| Point::new(x, 0.0)), false)
}

/// tlprocedural's `extractSubPath(points, start, end, allowWrap)` on the open polyline `sp`, cut
/// the way [`trim::trim`] cuts (a closed subpath allows the wrap; the polyline has no closing
/// segment here, as tlprocedural's tests' points haven't).
fn extract(sp: &SubPath, start: f64, end: f64, allow_wrap: bool) -> Vec<Point> {
    let m = Measured::new(pieces(sp));
    let Some((s, e)) = trim::window(start, end, allow_wrap) else { return pts(sp) };
    let cut = if s < e {
        m.cut(s * m.total, e * m.total)
    } else {
        let mut a = m.cut(s * m.total, m.total);
        // tlprocedural's polyline joins the halves (end point → start point) with a segment; a
        // closed subpath's closing segment is that join.
        if let (Some(last), Some(first)) = (a.last().map(|p| p.c.p3), sp.anchors.first().map(|a| a.p)) {
            a.push(Piece::line(last, first));
        }
        a.extend(m.cut(0.0, e * m.total));
        a
    };
    pts(&from_pieces(&cut, false))
}

#[test]
fn extract_normal_range_includes_interior_vertices() {
    let p = extract(&line(), 0.25, 0.75, true);
    assert!(near(p[0], Point::new(10.0, 0.0)));
    assert!(near(*p.last().unwrap(), Point::new(30.0, 0.0)));
    assert!(xs(&p).contains(&20.0));
}

#[test]
fn extract_wrap_around_emits_both_halves() {
    let p = extract(&line(), 0.8, 0.2, true);
    assert!(p.len() >= 2);
    assert!(near(p[0], Point::new(32.0, 0.0)));
    assert!(near(*p.last().unwrap(), Point::new(8.0, 0.0)));
    let xs = xs(&p);
    assert!(xs.contains(&40.0) && xs.contains(&0.0), "{xs:?}");
}

#[test]
fn extract_wrap_around_at_symmetrical_point_nine_to_point_one() {
    let p = extract(&line(), 0.9, 0.1, true);
    assert!(near(p[0], Point::new(36.0, 0.0)));
    assert!(near(*p.last().unwrap(), Point::new(4.0, 0.0)));
}

#[test]
fn extract_full_range_spans_the_whole_line() {
    assert!(extract(&line(), 0.0, 1.0, true).len() >= 2);
}

#[test]
fn extract_start_equal_end_returns_the_whole_path() {
    assert!(extract(&line(), 0.5, 0.5, true).len() >= 2);
    assert_eq!(trim::trim(&line(), &Trim { start: 0.5, end: 0.5, offset: 0.0 }), trim::Trimmed::Whole);
}

#[test]
fn extract_handles_too_few_points() {
    let one = SubPath::polyline(&[Point::ZERO], false);
    assert_eq!(trim::trim(&one, &Trim::default()), trim::Trimmed::Whole);
}

#[test]
fn extract_open_path_does_not_wrap() {
    let p = extract(&line(), 0.8, 0.2, false);
    assert!(near(p[0], Point::new(8.0, 0.0)));
    assert!(near(*p.last().unwrap(), Point::new(32.0, 0.0)));
    let xs = xs(&p);
    assert!(!xs.contains(&0.0) && !xs.contains(&40.0), "{xs:?}");
    // The row itself: an open path never wraps.
    let trim::Trimmed::Part(t) = trim::trim(&line(), &Trim { start: 0.8, end: 0.2, offset: 0.0 }) else { panic!() };
    assert_eq!(xs_of(&t), vec![8.0, 10.0, 20.0, 30.0, 32.0]);
}

#[test]
fn extract_offset_shifted_range_wraps() {
    // start 0.6 + offset 0.3 = 0.9; end 0.8 + 0.3 = 1.1 → 0.1.
    let p = extract(&line(), 0.9, 1.1, true);
    assert!(near(p[0], Point::new(36.0, 0.0)));
    assert!(near(*p.last().unwrap(), Point::new(4.0, 0.0)));
}

fn xs_of(sp: &SubPath) -> Vec<f64> {
    xs(&pts(sp))
}

// ---- Trim: new ----

#[test]
fn trim_a_closed_square_wraps_through_its_start_and_opens() {
    // A 100 × 100 square from (0,0) clockwise: perimeter 400.
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).subpaths[0].clone();
    assert!(sq.closed);
    // offset 0.5 slides the default window (0.1..0.9) to 0.6..1.4 → 0.6..0.4 through the start.
    let trim::Trimmed::Part(t) = trim::trim(&sq, &Trim { start: 0.1, end: 0.9, offset: 0.5 }) else { panic!() };
    assert!(!t.closed);
    let m = Measured::new(pieces(&t));
    assert!((m.total - 320.0).abs() < 1e-6, "{}", m.total);
    // Through the first anchor.
    assert!(pts(&t).iter().any(|p| near(*p, sq.anchors[0].p)));
}

#[test]
fn trim_keeps_curves_exactly() {
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).subpaths[0].clone();
    let whole = Measured::new(pieces(&circle)).total;
    let trim::Trimmed::Part(half) = trim::trim(&circle, &Trim { start: 0.0, end: 0.5, offset: 0.0 }) else { panic!() };
    let m = Measured::new(pieces(&half));
    assert!((m.total - whole / 2.0).abs() < 1e-3, "{} vs {}", m.total, whole / 2.0);
    assert!(pieces(&half).iter().all(|p| !p.line), "curves stay curves");
    for a in &half.anchors {
        assert!((a.p.distance(Point::new(100.0, 100.0)) - 100.0).abs() < 0.2, "on the circle: {:?}", a.p);
    }
}

#[test]
fn trim_window_that_is_empty_drops_the_subpath() {
    let p = PathData::new(vec![line()]);
    let tiny = super::shape_path(&p, &GeometryOp::Trim(Trim { start: 0.5, end: 0.5 + 1e-12, offset: 0.0 }));
    assert_eq!(tiny, p, "a window below the precision is the whole path (start = end)");
}

// ---- Resample: subdivide ----

fn square() -> SubPath {
    shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).subpaths[0].clone()
}

fn subdivide(method: SubdivideMethod, iterations: u32) -> Resample {
    Resample { mode: ResampleMode::Subdivide, method, iterations, ..Resample::default() }
}

/// tlprocedural's `subdivideRingOnce` comment: "a 4-gon would yield 10 points, not 8" was the bug
/// it fixed: a closed 4-gon doubles to 8, an open path of n to 2n − 1.
#[test]
fn subdivide_doubles_a_four_gon_to_eight() {
    for method in [SubdivideMethod::Linear, SubdivideMethod::CatmullRom, SubdivideMethod::Chaikin] {
        let r = resample::resample(&square(), &subdivide(method, 1)).unwrap();
        assert_eq!(r.anchors.len(), 8, "{method:?}");
        assert_eq!(resample::count(&square(), &subdivide(method, 1)), Some(8));
        let open = SubPath::new(square().anchors, false);
        assert_eq!(resample::resample(&open, &subdivide(method, 1)).unwrap().anchors.len(), 7, "{method:?}");
        assert_eq!(resample::count(&open, &subdivide(method, 1)), Some(7));
    }
    // Three iterations: 4 → 32.
    assert_eq!(resample::resample(&square(), &subdivide(SubdivideMethod::Linear, 3)).unwrap().anchors.len(), 32);
}

#[test]
fn linear_subdivision_inserts_midpoints_and_keeps_curves() {
    let r = resample::resample(&square(), &subdivide(SubdivideMethod::Linear, 1)).unwrap();
    assert!(near(r.anchors[0].p, square().anchors[0].p));
    assert!(near(r.anchors[1].p, square().anchors[0].p.midpoint(square().anchors[1].p)));
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).subpaths[0].clone();
    let r = resample::resample(&circle, &subdivide(SubdivideMethod::Linear, 2)).unwrap();
    assert_eq!(r.anchors.len(), circle.anchors.len() * 4);
    let (a, b) = (Measured::new(pieces(&circle)).total, Measured::new(pieces(&r)).total);
    assert!((a - b).abs() < 1e-3, "same curve: {a} vs {b}");
}

#[test]
fn chaikin_cuts_corners_at_the_quarter_points() {
    let r = resample::resample(&square(), &subdivide(SubdivideMethod::Chaikin, 1)).unwrap();
    let (a, b) = (square().anchors[0].p, square().anchors[1].p);
    assert!(near(r.anchors[0].p, a.lerp(b, 0.25)));
    assert!(near(r.anchors[1].p, a.lerp(b, 0.75)));
}

#[test]
fn catmull_rom_keeps_the_anchors_and_adds_spline_midpoints() {
    let r = resample::resample(&square(), &subdivide(SubdivideMethod::CatmullRom, 1)).unwrap();
    for (k, a) in square().anchors.iter().enumerate() {
        assert!(near(r.anchors[2 * k].p, a.p));
    }
    // The spline bulges out past the square's side midpoint.
    let (a, b) = (square().anchors[0].p, square().anchors[1].p);
    assert!(r.anchors[1].p.distance(a.midpoint(b)) > 1.0);
}

// ---- Resample: rebuild by count and by spacing (tlprocedural's `resamplePolyline`) ----

#[test]
fn count_on_a_closed_path_spaces_points_evenly_round_it() {
    let r = resample::resample(&square(), &Resample { mode: ResampleMode::Count, count: 8, ..Resample::default() }).unwrap();
    assert!(r.closed);
    assert_eq!(r.anchors.len(), 8);
    // Every 50 points round the 400-point perimeter.
    let m = Measured::new(pieces(&r));
    assert!((m.total - 400.0).abs() < 1e-6);
    for w in r.anchors.windows(2) {
        assert!((w[0].p.distance(w[1].p) - 50.0).abs() < 1e-6);
    }
    // A closed subpath gets at least three.
    let r = resample::resample(&square(), &Resample { mode: ResampleMode::Count, count: 2, ..Resample::default() }).unwrap();
    assert_eq!(r.anchors.len(), 3);
}

#[test]
fn count_on_an_open_path_keeps_both_ends() {
    let r = resample::resample(&line(), &Resample { mode: ResampleMode::Count, count: 3, ..Resample::default() }).unwrap();
    assert_eq!(xs_of(&r), vec![0.0, 20.0, 40.0]);
}

#[test]
fn spacing_steps_from_the_start_and_ends_at_the_end() {
    let r = resample::resample(&line(), &Resample { mode: ResampleMode::Spacing, spacing: 15.0, ..Resample::default() }).unwrap();
    assert_eq!(xs_of(&r), vec![0.0, 15.0, 30.0, 40.0]);
    let r = resample::resample(&square(), &Resample { mode: ResampleMode::Spacing, spacing: 30.0, ..Resample::default() }).unwrap();
    // ceil(400 / 30) = 14 points, 30 apart along the path, the last gap shorter.
    assert_eq!(r.anchors.len(), 14);
    assert_eq!(resample::count(&square(), &Resample { mode: ResampleMode::Spacing, spacing: 30.0, ..Resample::default() }), Some(14));
}

#[test]
fn count_follows_the_true_curve() {
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).subpaths[0].clone();
    let r = resample::resample(&circle, &Resample { mode: ResampleMode::Count, count: 12, ..Resample::default() }).unwrap();
    assert_eq!(r.anchors.len(), 12);
    for a in &r.anchors {
        assert!((a.p.distance(Point::new(100.0, 100.0)) - 100.0).abs() < 0.1, "{:?}", a.p);
    }
}

// ---- Tidy: tlprocedural's Close Path and Sort Points ----

fn open_v() -> SubPath {
    SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(50.0, 100.0), Point::new(100.0, 0.0)], false)
}

#[test]
fn close_straight_adds_the_closing_segment() {
    let t = tidy::tidy(&open_v(), &Tidy::default()).unwrap();
    assert!(t.closed);
    assert_eq!(t.anchors.len(), 3);
    assert!(pieces(&t).last().unwrap().line);
    // Already closed: nothing to do.
    assert_eq!(tidy::tidy(&square(), &Tidy::default()), None);
}

#[test]
fn close_joins_ends_within_the_tolerance() {
    let mut p = pts(&open_v());
    p.push(Point::new(0.5, 0.0));
    let sp = SubPath::polyline(&p, false);
    let t = tidy::tidy(&sp, &Tidy { tolerance: 1.0, ..Tidy::default() }).unwrap();
    assert!(t.closed);
    assert_eq!(t.anchors.len(), 3, "the near end became the first anchor");
    let t = tidy::tidy(&sp, &Tidy { tolerance: 0.1, ..Tidy::default() }).unwrap();
    assert_eq!(t.anchors.len(), 4);
}

#[test]
fn close_smooth_carries_on_both_ends() {
    let t = tidy::tidy(&open_v(), &Tidy { close: CloseMethod::Smooth, ..Tidy::default() }).unwrap();
    let closing = *pieces(&t).last().unwrap();
    assert!(!closing.line);
    // Leaving (100, 0) the way the last segment arrived (down and right).
    let d = closing.c.p1 - closing.c.p0;
    assert!(d.x > 0.0 && d.y < 0.0, "{d:?}");
    // Off: left open.
    assert_eq!(tidy::tidy(&open_v(), &Tidy { close: CloseMethod::Off, ..Tidy::default() }), None);
}

#[test]
fn sort_by_x_and_y_and_reverse() {
    let sp = SubPath::polyline(&[Point::new(30.0, 5.0), Point::new(10.0, 9.0), Point::new(20.0, 1.0)], false);
    let by = |sort, reverse| pts(&tidy::tidy(&sp, &Tidy { close: CloseMethod::Off, sort, reverse, ..Tidy::default() }).unwrap());
    assert_eq!(xs(&by(SortBy::X, false)), vec![10.0, 20.0, 30.0]);
    assert_eq!(xs(&by(SortBy::X, true)), vec![30.0, 20.0, 10.0]);
    assert_eq!(by(SortBy::Y, false).iter().map(|p| p.y).collect::<Vec<_>>(), vec![1.0, 5.0, 9.0]);
}

#[test]
fn sort_by_angle_goes_round_the_centre() {
    // A square's corners given out of order.
    let sp = SubPath::polyline(&[Point::new(1.0, 1.0), Point::new(-1.0, -1.0), Point::new(1.0, -1.0), Point::new(-1.0, 1.0)], true);
    let t = tidy::tidy(&sp, &Tidy { sort: SortBy::Angle, ..Tidy::default() }).unwrap();
    assert!(t.closed);
    let angles: Vec<f64> = pts(&t).iter().map(|p| p.y.atan2(p.x)).collect();
    assert!(angles.windows(2).all(|w| w[0] <= w[1]), "{angles:?}");
    let d = tidy::tidy(&sp, &Tidy { sort: SortBy::Distance, ..Tidy::default() }).unwrap();
    assert_eq!(d.anchors.len(), 4);
}

#[test]
fn random_sort_is_seeded() {
    let sp = SubPath::polyline(&(0..20).map(|i| Point::new(f64::from(i), 0.0)).collect::<Vec<_>>(), false);
    let shuffle = |seed| pts(&tidy::tidy(&sp, &Tidy { close: CloseMethod::Off, sort: SortBy::Random, seed, ..Tidy::default() }).unwrap());
    assert_eq!(shuffle(3), shuffle(3));
    assert_ne!(shuffle(3), shuffle(4));
    let mut sorted = xs(&shuffle(3));
    sorted.sort_by(f64::total_cmp);
    assert_eq!(sorted, xs(&pts(&sp)), "a permutation");
}

#[test]
fn reverse_without_sort_reverses_the_direction_keeping_curves() {
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).subpaths[0].clone();
    let r = tidy::tidy(&circle, &Tidy { reverse: true, ..Tidy::default() }).unwrap();
    assert_eq!(r.anchors.len(), circle.anchors.len());
    let (a, b) = (Measured::new(pieces(&circle)).total, Measured::new(pieces(&r)).total);
    assert!((a - b).abs() < 1e-6);
    assert_eq!(tidy::reversed(&r), circle, "twice is the original");
    // Neither sorting nor reversing (and closed): nothing to do.
    assert_eq!(tidy::tidy(&circle, &Tidy::default()), None);
}

// ---- Rows in the stack ----

fn stack(rows: Value) -> crate::Stack {
    parse(&json!({"v": 1, "stack": rows}).to_string()).unwrap()
}

fn path_node(sp: SubPath) -> Node {
    Node::path(NodeId(1), PathData::new(vec![sp]), Appearance::default_art())
}

#[test]
fn geometry_rows_run_on_the_source_above_copies_and_on_copies_below() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    let out = evaluate(
        &stack(json!([
            {"id": "a", "kind": "resample"},
            {"id": "b", "kind": "linearArray", "count": 3},
            {"id": "c", "kind": "trim"},
        ])),
        b,
    );
    assert_eq!(out.instances.len(), 3);
    let at: Vec<(&str, RowAt)> = out.geometry.iter().map(|g| (g.modifier.as_str(), g.at)).collect();
    assert_eq!(at, vec![("a", RowAt::Source), ("c", RowAt::Copies)]);
    // Only geometry rows: one unmoved copy, still to be built.
    let out = evaluate(&stack(json!([{"id": "a", "kind": "trim"}])), b);
    assert_eq!(out.instances.len(), 1);
    assert!(out.has_geometry());
    // Disabled rows and rows below a stencil don't run.
    let out =
        evaluate(&stack(json!([{"id": "a", "kind": "trim", "enabled": false}, {"id": "s", "kind": "stencil"}, {"id": "t", "kind": "tidy"}])), b);
    assert!(!out.has_geometry());
}

#[test]
fn junk_params_are_clamped_with_warnings() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    let out = evaluate(
        &stack(json!([
            {"id": "a", "kind": "resample", "mode": "subdivide", "iterations": 99, "count": 0, "spacing": -5},
            {"id": "b", "kind": "trim", "start": -3, "end": "7", "offset": 1e300},
            {"id": "c", "kind": "tidy", "tolerance": 1e9, "seed": 5e9},
        ])),
        b,
    );
    let ops: Vec<&GeometryOp> = out.geometry.iter().map(|g| &g.op).collect();
    let GeometryOp::Resample(r) = ops[0] else { panic!() };
    assert_eq!((r.iterations, r.count, r.spacing), (5, 2, 1.0));
    let GeometryOp::Trim(t) = ops[1] else { panic!() };
    assert_eq!((t.start, t.end, t.offset), (0.0, 1.0, 1.0));
    let GeometryOp::Tidy(t) = ops[2] else { panic!() };
    assert_eq!((t.tolerance, t.seed), (500.0, 999_999));
    assert!(out.warnings.len() >= 7, "{:?}", out.warnings);
    // Wrong types don't read: the row is kept but ignored (as every kind's).
    let out = evaluate(&stack(json!([{"id": "a", "kind": "resample", "mode": "wobble"}])), b);
    assert!(!out.has_geometry());
    assert!(!out.warnings.is_empty());
}

#[test]
fn shaper_runs_rows_on_paths_in_groups_and_compounds_and_notes_the_rest() {
    let out = evaluate(&stack(json!([{"id": "a", "kind": "resample", "count": 10}])), Rect::new(0.0, 0.0, 100.0, 100.0));
    let mut live = path_node(square());
    if let NodeKind::Path { live: l, .. } = &mut live.kind {
        *l = Some(Box::new(vectorcraft_doc::LiveShape::Ellipse { w: 100.0, h: 100.0, pie: (0.0, 360.0), xf: vectorcraft_geom::Affine::IDENTITY }));
    }
    let compound = Node {
        kind: NodeKind::Compound { children: vec![std::sync::Arc::new(path_node(square()))], rule: Default::default() },
        ..path_node(square())
    };
    let text = Node { kind: NodeKind::SymbolInstance { symbol: "s".into(), xf: vectorcraft_geom::Affine::IDENTITY }, ..path_node(square()) };
    let mut g = Node::group(NodeId(2), vec![std::sync::Arc::new(live), std::sync::Arc::new(compound), std::sync::Arc::new(text)]);
    let mut sh = Shaper::new(&out);
    sh.source(&mut g);
    let kids = g.children().unwrap();
    let NodeKind::Path { path, live, .. } = &kids[0].kind else { panic!() };
    assert_eq!(path.anchor_count(), 10);
    assert!(live.is_none(), "the live shape became a plain path");
    let inner = kids[1].children().unwrap()[0].path_data().unwrap();
    assert_eq!(inner.anchor_count(), 10);
    assert_eq!(sh.notes.len(), 1, "{:?}", sh.notes);
    assert!(sh.notes[0].contains("symbols"));
    // Copy rows: none here.
    let before = g.clone();
    sh.copy(&mut g);
    assert_eq!(g, before);
}

#[test]
fn the_anchor_cap_skips_a_row_that_would_pass_it() {
    // 1000 copies share the budget: 200 anchors each; 1024 per copy is too many.
    let out = evaluate(
        &stack(json!([{"id": "a", "kind": "linearArray", "count": 1000}, {"id": "b", "kind": "resample", "count": 1024}])),
        Rect::new(0.0, 0.0, 100.0, 100.0),
    );
    let mut sh = Shaper::new(&out);
    let mut n = path_node(square());
    sh.copy(&mut n);
    assert_eq!(n.path_data().unwrap().anchor_count(), 4, "skipped");
    assert!(sh.notes[0].contains(&MAX_GEOMETRY_ANCHORS.to_string()), "{:?}", sh.notes);
    // Spacing of 1 point round a huge path is refused before anything is made.
    let out = evaluate(&stack(json!([{"id": "b", "kind": "resample", "mode": "spacing", "spacing": 1}])), Rect::new(0.0, 0.0, 1.0, 1.0));
    let mut sh = Shaper::new(&out);
    let mut n = path_node(shapes::rectangle(Rect::new(0.0, 0.0, 1.0e6, 1.0e6)).subpaths[0].clone());
    sh.source(&mut n);
    assert_eq!(n.path_data().unwrap().anchor_count(), 4);
    assert_eq!(sh.notes.len(), 1);
    // A trim on many anchors is never refused (it adds at most two).
    let out =
        evaluate(&stack(json!([{"id": "a", "kind": "linearArray", "count": 1000}, {"id": "b", "kind": "trim"}])), Rect::new(0.0, 0.0, 100.0, 100.0));
    let big = SubPath::polyline(&(0..500).map(|i| Point::new(f64::from(i), 0.0)).collect::<Vec<_>>(), false);
    let mut sh = Shaper::new(&out);
    let mut n = path_node(big);
    sh.copy(&mut n);
    assert!(sh.notes.is_empty(), "{:?}", sh.notes);
    assert!(n.path_data().unwrap().anchor_count() < 500);
}

#[test]
fn junk_geometry_never_panics_and_stays_finite() {
    let weird = [
        SubPath::new(vec![], false),
        SubPath::new(vec![], true),
        SubPath::polyline(&[Point::ZERO], true),
        SubPath::polyline(&[Point::ZERO, Point::ZERO, Point::ZERO], true),
        SubPath::polyline(&[Point::new(1e300, -1e300), Point::new(-1e300, 1e300)], false),
        SubPath::new(vec![Anchor::with_handles(Point::ZERO, Point::new(5.0, 5.0), Point::new(-5.0, 5.0)), Anchor::corner(Point::ZERO)], true),
        SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)], false),
    ];
    let ops: Vec<GeometryOp> = vec![
        GeometryOp::Resample(Resample::default()),
        GeometryOp::Resample(Resample { mode: ResampleMode::Spacing, spacing: 1.0, ..Resample::default() }),
        GeometryOp::Resample(subdivide(SubdivideMethod::Linear, 5)),
        GeometryOp::Resample(subdivide(SubdivideMethod::CatmullRom, 5)),
        GeometryOp::Resample(subdivide(SubdivideMethod::Chaikin, 5)),
        GeometryOp::Trim(Trim::default()),
        GeometryOp::Trim(Trim { start: 0.9, end: 0.1, offset: 0.3 }),
        GeometryOp::Tidy(Tidy { close: CloseMethod::Smooth, sort: SortBy::Angle, reverse: true, ..Tidy::default() }),
        GeometryOp::Tidy(Tidy { sort: SortBy::Random, ..Tidy::default() }),
    ];
    for sp in &weird {
        for op in &ops {
            let rows = vec![super::GeometryRow { modifier: "m".into(), op: op.clone(), at: RowAt::Source }];
            let out = Output { instances: vec![crate::Instance::identity()], geometry: rows, ..Output::default() };
            let mut sh = Shaper::new(&out);
            let mut n = path_node(sp.clone());
            sh.source(&mut n);
            let ok = n.path_data().unwrap().subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite());
            let was_ok = sp.anchors.iter().all(|a| a.p.is_finite());
            assert!(ok || !was_ok, "{op:?} on {sp:?}");
        }
    }
}
