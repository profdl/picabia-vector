// Tests may unwrap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, shapes};

use super::{GeometryOp, ScreenRow, Shaper, shaped_copies};
use crate::model::{ModifierKind, Screen, SpotShape, ToneSource};
use crate::screen::ToneImage;
use crate::target::ToneTarget;
use crate::{Output, evaluate, parse};

fn stack(rows: Value) -> crate::Stack {
    parse(&json!({"v": 1, "stack": rows}).to_string()).unwrap()
}

fn node(path: PathData, fill: Paint) -> Node {
    Node::path(NodeId(1), path, Appearance::basic(fill, Paint::solid(Color::BLACK), 2.0))
}

fn black() -> Paint {
    Paint::solid(Color::BLACK)
}

fn square(side: f64) -> PathData {
    shapes::rectangle(Rect::new(0.0, 0.0, side, side))
}

/// `n` after the stack `rows` (on the object: no copy rows), and what it said.
fn screened(n: &Node, rows: Value) -> (Node, Vec<String>) {
    let out = evaluate(&stack(rows), n.geometric_bounds().unwrap_or(Rect::ZERO));
    let mut m = n.clone();
    let mut s = Shaper::new(&out);
    s.source(&mut m);
    (m, s.notes)
}

/// `n` after one screen row built by hand (an image resolved, or junk the evaluator would clamp).
fn screened_with(n: &Node, row: ScreenRow) -> (Node, Vec<String>) {
    let out = Output {
        instances: vec![crate::Instance::identity()],
        geometry: vec![super::GeometryRow { modifier: "s".into(), op: GeometryOp::Screen(row), at: super::RowAt::Source }],
        ..Output::default()
    };
    let mut m = n.clone();
    let mut s = Shaper::new(&out);
    s.source(&mut m);
    (m, s.notes)
}

fn dots(n: &Node) -> &PathData {
    n.path_data().expect("a path")
}

/// The inked area of `n`'s path (by its fill rule).
fn inked(n: &Node) -> f64 {
    let NodeKind::Path { path, rule, .. } = &n.kind else { panic!("a path") };
    vectorcraft_pathops::area(path, *rule)
}

fn fixed(spot: &str, amount: f64) -> Value {
    json!([{"id": "s", "kind": "screen", "spot": spot, "tone": "fixed", "amount": amount, "angle": 30}])
}

#[test]
fn defaults_presets_and_ranges() {
    let ModifierKind::Screen(s) = crate::Modifier::new("m", "screen").unwrap().kind else { panic!() };
    assert_eq!((s.spot, s.lpi, s.angle, s.quality, s.tone, s.amount, s.invert), (SpotShape::Round, 20.0, 45.0, 12, ToneSource::Fill, 50.0, false));
    assert!(s.image.is_empty());
    let params = ModifierKind::Screen(s).params();
    for key in ["spot", "lpi", "angle", "quality", "tone", "amount", "image", "invert"] {
        assert!(params.contains_key(key), "{key} in {params:?}");
    }
    assert_eq!((params["spot"].as_str(), params["tone"].as_str()), (Some("round"), Some("fill")));
    // The presets: fixed tones, round dots and lines.
    let ben = Screen::preset("benDay").unwrap();
    assert_eq!((ben.spot, ben.tone, ben.amount, ben.angle, ben.lpi), (SpotShape::Round, ToneSource::Fixed, 30.0, 45.0, 10.0));
    let lines = Screen::preset("lineFill").unwrap();
    assert_eq!((lines.spot, lines.tone, lines.amount), (SpotShape::Line, ToneSource::Fixed, 30.0));
    assert!(Screen::preset("halftone").is_none());
    assert_eq!(crate::SCREEN_PRESETS.iter().map(|p| p.0).collect::<Vec<_>>(), ["benDay", "lineFill"]);
    // Junk clamped, each with a warning.
    let out = evaluate(
        &stack(json!([{"id": "s", "kind": "screen", "lpi": 1e9, "angle": -5000, "quality": 0, "amount": 400}])),
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    let GeometryOp::Screen(row) = &out.geometry[0].op else { panic!() };
    let p = &row.params;
    assert_eq!((p.lpi, p.angle, p.quality, p.amount), (300.0, -360.0, 4, 100.0));
    assert!(out.warnings.len() >= 4, "{:?}", out.warnings);
    // A spot or tone that isn't one doesn't read: the row is kept but ignored.
    for bad in [json!({"spot": "star"}), json!({"tone": "loud"}), json!({"lpi": "many"}), json!({"image": [1]})] {
        let mut row = json!({"id": "s", "kind": "screen"});
        row.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        let out = evaluate(&stack(json!([row])), Rect::new(0.0, 0.0, 1.0, 1.0));
        assert!(!out.has_geometry(), "{bad}");
    }
    // A numeric image id reads as text.
    let s = stack(json!([{"id": "s", "kind": "screen", "tone": "image", "image": 12}]));
    let ModifierKind::Screen(p) = &s.modifiers[0].kind else { panic!() };
    assert_eq!(p.image, "12");
    assert_eq!(s.targets().collect::<Vec<_>>(), [("s", "12")]);
    assert_eq!(s.screen_images().collect::<Vec<_>>(), ["12"]);
}

#[test]
fn a_fixed_tone_inks_that_share_of_the_shape_for_every_spot() {
    let n = node(square(200.0), black());
    for spot in ["round", "ellipse", "line", "square", "diamond", "cross"] {
        for amount in [10.0, 30.0, 50.0, 80.0] {
            // A cross at 10% has arms 5% of a cell wide, under one sample at the default quality
            // (12): it needs a finer quality (below).
            if spot == "cross" && amount < 20.0 {
                continue;
            }
            let (m, notes) = screened(&n, fixed(spot, amount));
            assert!(notes.is_empty(), "{spot} {amount}: {notes:?}");
            let share = inked(&m) / 40_000.0;
            assert!((share - amount / 100.0).abs() < 0.03, "{spot} at {amount}%: {share}");
            // Filled, no stroke; a plain path now.
            let a = &m.appearance;
            assert_eq!(a.fill_paint(), black());
            assert!(a.stroke().is_none());
            assert!(matches!(&m.kind, NodeKind::Path { live: None, rule: FillRule::NonZero, .. }));
        }
    }
    // A light cross traced finely enough.
    let (m, _) = screened(&n, json!([{"id": "s", "kind": "screen", "spot": "cross", "tone": "fixed", "amount": 10, "quality": 48}]));
    assert!((inked(&m) / 40_000.0 - 0.1).abs() < 0.03, "{}", inked(&m) / 40_000.0);
    // Round dots are curves, not polygons.
    let (m, _) = screened(&n, fixed("round", 20.0));
    let p = dots(&m);
    let curved = p.subpaths.iter().flat_map(|s| &s.anchors).filter(|a| a.h_in != a.p || a.h_out != a.p).count();
    assert!(curved * 2 > p.anchor_count(), "{curved} of {}", p.anchor_count());
    assert!(p.anchor_count() <= p.subpaths.len() * 8, "few anchors a dot: {} for {}", p.anchor_count(), p.subpaths.len());
}

#[test]
fn the_dots_are_clipped_to_the_outline() {
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0));
    for amount in [30.0, 50.0, 90.0] {
        let (m, notes) = screened(&node(circle.clone(), black()), fixed("round", amount));
        assert!(notes.is_empty(), "{notes:?}");
        let c = Point::new(100.0, 100.0);
        for a in dots(&m).subpaths.iter().flat_map(|s| &s.anchors) {
            assert!((a.p - c).hypot() <= 100.0 + 0.05, "{:?} outside", a.p);
        }
        let share = inked(&m) / (std::f64::consts::PI * 100.0 * 100.0);
        assert!((share - amount / 100.0).abs() < 0.03, "{amount}%: {share}");
    }
    // A ring (even-odd compound outline): nothing in the hole.
    let mut donut = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0));
    donut.subpaths.extend(shapes::ellipse(Rect::new(50.0, 50.0, 150.0, 150.0)).subpaths);
    let mut n = node(donut, black());
    if let NodeKind::Path { rule, .. } = &mut n.kind {
        *rule = FillRule::EvenOdd;
    }
    let (m, _) = screened(&n, fixed("line", 50.0));
    for a in dots(&m).subpaths.iter().flat_map(|s| &s.anchors) {
        assert!((a.p - Point::new(100.0, 100.0)).hypot() >= 50.0 - 0.05, "{:?} in the hole", a.p);
    }
    let ring = std::f64::consts::PI * (100.0f64.powi(2) - 50.0f64.powi(2));
    assert!((inked(&m) / ring - 0.5).abs() < 0.04, "{}", inked(&m) / ring);
}

#[test]
fn the_fill_gives_the_tone_solid_and_gradient() {
    let rows = json!([{"id": "s", "kind": "screen"}]);
    let n = node(square(100.0), black());
    let (m, _) = screened(&n, rows.clone());
    assert!(inked(&m) / 10_000.0 > 0.97, "black is solid ink: {}", inked(&m));
    let (m, _) = screened(&node(square(100.0), Paint::solid(Color::WHITE)), rows.clone());
    assert!(inked(&m) < 1.0, "white is no ink: {}", inked(&m));
    // Mid grey: half.
    let (m, _) = screened(&node(square(200.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5))), rows.clone());
    assert!((inked(&m) / 40_000.0 - 0.5).abs() < 0.03, "{}", inked(&m) / 40_000.0);
    // The fill gives the tone only: the dots are black (the lead's decision, PIC20.5d), whatever
    // colour they read.
    let (m, _) = screened(&node(square(200.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0))), rows.clone());
    assert_eq!(m.appearance.fill_paint(), black());
    // A black-to-white linear gradient left to right: dense on the left, sparse on the right, in
    // black.
    let g = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)]));
    let n = node(square(200.0), Paint::Gradient(Box::new(g.clone())));
    let (m, notes) = screened(&n, rows);
    assert!(notes.is_empty(), "{notes:?}");
    let half = |x0: f64| {
        let clip = shapes::rectangle(Rect::new(x0, 0.0, x0 + 100.0, 200.0));
        let p = vectorcraft_pathops::boolean(dots(&m), FillRule::NonZero, &clip, FillRule::NonZero, vectorcraft_pathops::BoolOp::Intersect);
        vectorcraft_pathops::area(&p, FillRule::NonZero) / 20_000.0
    };
    let (left, right) = (half(0.0), half(100.0));
    assert!((left - 0.75).abs() < 0.05 && (right - 0.25).abs() < 0.05, "{left} {right}");
    assert_eq!(m.appearance.fill_paint(), black());
    // A fixed tone inks in the fill: the gradient stays where it was on the outline's box (not
    // refitted to the dots' smaller box).
    let outline = Rect::new(0.0, 0.0, 200.0, 200.0);
    for p in [square(200.0), shapes::ellipse(outline)] {
        let n = node(p, Paint::Gradient(Box::new(g.clone())));
        let (m, _) = screened(&n, fixed("round", 30.0));
        let Paint::Gradient(kept) = m.appearance.fill_paint() else { panic!("the gradient kept") };
        assert_eq!(kept.geom, Some(g.resolve(outline)), "pinned to the outline's box");
    }
}

#[test]
fn a_detailed_outline_stays_within_the_band_and_work_caps() {
    // A comb: 60 000 teeth, each a tall spike across the whole height (every piece spans every band
    // at sqrt(N) bands: the slots would be N × sqrt(N) without the cap).
    let mut pts = Vec::new();
    let teeth = 60_000;
    for k in 0..teeth {
        let x = k as f64 * 0.01;
        pts.push(Point::new(x, 0.0));
        pts.push(Point::new(x + 0.005, 400.0));
    }
    pts.push(Point::new(teeth as f64 * 0.01, 400.0));
    pts.push(Point::new(0.0, 400.0));
    let comb = PathData::new(vec![vectorcraft_geom::SubPath::polyline(&pts, true)]);
    let slots = super::halftone::band_slots(&comb, FillRule::NonZero, 0.05).unwrap();
    assert!(slots <= super::MAX_BAND_ENTRIES, "{slots}");
    // Screened at 1 lpi (few cells, so it passes the anchor estimate): over the segment cap for
    // the edge's boolean, skipped with a note at once, never a huge allocation.
    let n = node(comb, black());
    let t = std::time::Instant::now();
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "lpi": 1, "tone": "fixed", "amount": 50}]));
    assert!(t.elapsed().as_secs_f64() < 1.0, "{:?}", t.elapsed());
    assert_eq!(m, n);
    assert!(notes.iter().any(|s| s.contains("segments isn't screened")), "{notes:?}");
    // Just under the cap (a comb of 9 000 teeth): screened in bounded time.
    let mut pts = Vec::new();
    for k in 0..9_000 {
        let x = k as f64 * 0.05;
        pts.push(Point::new(x, 0.0));
        pts.push(Point::new(x + 0.025, 300.0));
    }
    pts.push(Point::new(450.0, 300.0));
    pts.push(Point::new(0.0, 300.0));
    let comb = PathData::new(vec![vectorcraft_geom::SubPath::polyline(&pts, true)]);
    let n = node(comb, black());
    let t = std::time::Instant::now();
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "lpi": 4, "tone": "fixed", "amount": 50}]));
    assert!(t.elapsed().as_secs_f64() < 2.0, "{:?}", t.elapsed());
    assert_eq!(m, n);
    assert!(notes.iter().any(|s| s.contains("too often to cut")), "{notes:?}");
    // A detailed but ordinary edge (a 4 000-sided polygon) is screened.
    let poly: Vec<Point> = (0..4000)
        .map(|k| {
            let a = k as f64 / 4000.0 * std::f64::consts::TAU;
            Point::new(150.0 + 140.0 * a.cos(), 150.0 + 140.0 * a.sin())
        })
        .collect();
    let n = node(PathData::new(vec![vectorcraft_geom::SubPath::polyline(&poly, true)]), black());
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "tone": "fixed", "amount": 50}]));
    assert!(notes.is_empty(), "{notes:?}");
    assert!((inked(&m) / (std::f64::consts::PI * 140.0 * 140.0) - 0.5).abs() < 0.03);
}

#[test]
fn invert_inks_the_light_parts() {
    let n = node(square(200.0), black());
    let mut rows = fixed("round", 30.0);
    rows[0]["invert"] = json!(true);
    let (m, _) = screened(&n, rows);
    assert!((inked(&m) / 40_000.0 - 0.7).abs() < 0.03, "{}", inked(&m) / 40_000.0);
}

/// PIC20.5e: a near-white ink (luminance over 0.9) would vanish on white paper: black instead. Just
/// under the threshold the colour stays; a gradient stays unless every stop is near-white.
#[test]
fn near_white_inks_become_black_and_light_colours_stay() {
    let ben = json!([{"id": "s", "kind": "screen", "preset": "benDay", "tone": "fixed", "amount": 30}]);
    let ink = |fill: Paint| screened(&node(square(100.0), fill), ben.clone()).0.appearance.fill_paint();
    assert_eq!(ink(Paint::solid(Color::WHITE)), black(), "a default white rectangle shows");
    // Grey 0.92 (luminance 0.92): black; grey 0.88: kept.
    assert_eq!(ink(Paint::solid(Color::rgb(0.92, 0.92, 0.92))), black());
    let grey = Paint::solid(Color::rgb(0.88, 0.88, 0.88));
    assert_eq!(ink(grey.clone()), grey);
    // Pale yellow (luminance 0.97): black; a light yellow-orange (0.85): kept.
    assert_eq!(ink(Paint::solid(Color::rgb(1.0, 1.0, 0.6))), black());
    let light = Paint::solid(Color::rgb(1.0, 0.85, 0.4));
    assert_eq!(ink(light.clone()), light);
    // Gradients: white to near-white becomes black; white to blue stays the gradient.
    let grad = |a: Color, b: Color| {
        Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, a), GradientStop::new(1.0, b)]))))
    };
    assert_eq!(ink(grad(Color::WHITE, Color::rgb(0.95, 0.95, 0.95))), black());
    assert!(matches!(ink(grad(Color::WHITE, Color::rgb(0.0, 0.0, 1.0))), Paint::Gradient(_)));
    // Stroke only, white: black too.
    let n = Node::path(NodeId(1), square(100.0), Appearance::basic(Paint::None, Paint::solid(Color::WHITE), 2.0));
    assert_eq!(screened(&n, ben).0.appearance.fill_paint(), black());
}

#[test]
fn a_stroked_shape_without_a_fill_is_screened_in_its_stroke_colour() {
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let n = Node::path(NodeId(1), square(100.0), Appearance::basic(Paint::None, red.clone(), 3.0));
    let (m, _) = screened(&n, fixed("round", 40.0));
    assert_eq!(m.appearance.fill_paint(), red);
    assert!(m.appearance.stroke().is_none());
    assert!(inked(&m) > 3000.0);
    // Neither fill nor stroke: black dots.
    let n = Node::path(NodeId(1), square(100.0), Appearance::basic(Paint::None, Paint::None, 0.0));
    let (m, _) = screened(&n, fixed("round", 40.0));
    assert_eq!(m.appearance.fill_paint(), black());
}

/// A 2 × 1 image, black on the left, white on the right, placed on `place`.
fn half_black(place: Rect) -> Arc<ToneTarget> {
    let image = ToneImage::from_luma8(2, 1, &[0, 255], Rect::new(0.0, 0.0, 2.0, 1.0)).unwrap();
    let xf = Affine::translate((place.x0, place.y0)) * Affine::scale_non_uniform(place.width() / 2.0, place.height());
    Arc::new(ToneTarget { image: Arc::new(image), to_pixels: xf.inverse(), key: 7 })
}

#[test]
fn an_image_gives_the_tone_where_it_sits_and_a_missing_one_skips_the_row() {
    let n = node(square(200.0), black());
    let params = Screen { tone: ToneSource::Image, image: "5".into(), ..Screen::default() };
    // Pixel centres at x = 50 (black) and 150 (white), bilinear between: dark on the left.
    let (m, notes) = screened_with(&n, ScreenRow { params: params.clone(), image: Some(Ok(half_black(Rect::new(0.0, 0.0, 200.0, 200.0)))) });
    assert!(notes.is_empty(), "{notes:?}");
    let left = Rect::new(0.0, 0.0, 50.0, 200.0);
    let right = Rect::new(150.0, 0.0, 200.0, 200.0);
    let inside = |r: Rect| dots(&m).subpaths.iter().filter(|s| s.anchors.iter().all(|a| r.contains(a.p))).count();
    let all = |r: Rect| {
        vectorcraft_pathops::area(
            &vectorcraft_pathops::boolean(
                dots(&m),
                FillRule::NonZero,
                &shapes::rectangle(r),
                FillRule::NonZero,
                vectorcraft_pathops::BoolOp::Intersect,
            ),
            FillRule::NonZero,
        )
    };
    assert!(all(left) / 10_000.0 > 0.95, "solid on the black side: {}", all(left));
    assert!(all(right) < 1.0 && inside(right) == 0, "nothing on the white side: {}", all(right));
    // Outside the image: no ink.
    let (m, _) = screened_with(&n, ScreenRow { params: params.clone(), image: Some(Ok(half_black(Rect::new(500.0, 500.0, 600.0, 600.0)))) });
    assert!(inked(&m) < 1.0);
    // The image gone: the row is skipped (the object as it was), with the resolver's reason.
    let (m, notes) = screened_with(&n, ScreenRow { params: params.clone(), image: Some(Err("the screen's image is gone; it is skipped".into())) });
    assert_eq!(m, n);
    assert_eq!(notes, ["Screen: the screen's image is gone; it is skipped"]);
    // Through the evaluator without a document, and with no image chosen.
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "tone": "image", "image": "5"}]));
    assert_eq!(m, n);
    assert!(notes.iter().any(|s| s.contains("needs the document")), "{notes:?}");
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "tone": "image"}]));
    assert_eq!(m, n);
    assert!(notes.iter().any(|s| s.contains("choose the image")), "{notes:?}");
}

#[test]
fn copies_below_an_array_are_screened_where_they_sit_on_one_page_screen() {
    // At 0° and 20 lpi the cells are 3.6 pt apart: copies 180 pt apart (50 cells) get the same
    // dots, moved; copies 137 pt apart (38.05 cells) do not.
    let n = node(square(100.0), black());
    let rows = |dx: f64| json!([{"id": "a", "kind": "linearArray", "count": 2, "offsetX": dx}, {"id": "s", "kind": "screen", "tone": "fixed", "amount": 30, "angle": 0}]);
    let out = evaluate(&stack(rows(180.0)), Rect::new(0.0, 0.0, 100.0, 100.0));
    let (copies, notes) = shaped_copies(&n, &out);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(copies.len(), 2);
    let pts = |c: &Node, dx: f64| -> Vec<(i64, i64)> {
        let mut v: Vec<(i64, i64)> = dots(c)
            .subpaths
            .iter()
            .flat_map(|s| &s.anchors)
            .map(|a| (((a.p.x - dx) * 100.0).round() as i64, (a.p.y * 100.0).round() as i64))
            .collect();
        v.sort_unstable();
        v
    };
    let (a, b) = (pts(&copies[0], 0.0), pts(&copies[1], 180.0));
    let same = a.iter().filter(|p| b.binary_search(p).is_ok()).count();
    assert!(same * 10 >= a.len() * 9, "the copies' dots line up: {same} of {}", a.len());
    let out = evaluate(&stack(rows(137.0)), Rect::new(0.0, 0.0, 100.0, 100.0));
    let (copies, _) = shaped_copies(&n, &out);
    let (a, b) = (pts(&copies[0], 0.0), pts(&copies[1], 137.0));
    let same = a.iter().filter(|p| b.binary_search(p).is_ok()).count();
    assert!(same * 2 < a.len(), "off the screen's period they differ: {same} of {}", a.len());
}

#[test]
fn over_the_anchor_cap_the_row_is_skipped_with_a_note() {
    let n = node(square(2000.0), black());
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "lpi": 300}]));
    assert_eq!(m, n);
    assert!(notes.iter().any(|s| s.contains("200000 anchors")), "{notes:?}");
}

#[test]
fn a_fine_quality_on_large_art_is_coarsened_with_a_note() {
    let n = node(square(1000.0), black());
    let (m, notes) = screened(&n, json!([{"id": "s", "kind": "screen", "lpi": 10, "quality": 64, "tone": "fixed", "amount": 40}]));
    assert!(notes.iter().any(|s| s.contains("more coarsely")), "{notes:?}");
    assert!((inked(&m) / 1.0e6 - 0.4).abs() < 0.03, "{}", inked(&m) / 1.0e6);
    assert!(dots(&m).anchor_count() <= super::MAX_GEOMETRY_ANCHORS);
}

#[test]
fn groups_compounds_and_other_objects() {
    let a = Arc::new(node(square(100.0), black()));
    let b = Arc::new(node(shapes::ellipse(Rect::new(200.0, 0.0, 300.0, 100.0)), Paint::solid(Color::WHITE)));
    let im = vectorcraft_doc::ImageObject {
        key: "k".into(),
        width: 2,
        height: 2,
        xf: Affine::IDENTITY,
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let text = Arc::new(Node::new(NodeId(9), NodeKind::Image(Box::new(im))));
    let g = Node::group(NodeId(2), vec![a, b, text]);
    let (m, notes) = screened(&g, json!([{"id": "s", "kind": "screen"}]));
    let kids = m.children().unwrap();
    assert!(inked(&kids[0]) > 9_000.0, "black member solid");
    assert!(inked(&kids[1]) < 1.0, "white member empty");
    assert!(notes.iter().any(|s| s.contains("paths only")), "{notes:?}");
    // A compound path paints as one: one member holding the dots, the compound's fill.
    let ring = vec![
        Arc::new(Node::path(NodeId(3), shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::default())),
        Arc::new(Node::path(NodeId(4), shapes::ellipse(Rect::new(50.0, 50.0, 150.0, 150.0)), Appearance::default())),
    ];
    let mut c = Node::new(NodeId(5), NodeKind::Compound { children: ring, rule: FillRule::EvenOdd });
    *c.appearance = Appearance::basic(black(), Paint::None, 0.0);
    let (m, _) = screened(&c, fixed("round", 50.0));
    let NodeKind::Compound { children, .. } = &m.kind else { panic!() };
    assert_eq!(children.len(), 1);
    let area = vectorcraft_pathops::area(children[0].path_data().unwrap(), FillRule::NonZero);
    let ring = std::f64::consts::PI * (100.0f64.powi(2) - 50.0f64.powi(2));
    assert!((area / ring - 0.5).abs() < 0.04, "{}", area / ring);
}

#[test]
fn junk_never_panics_and_stays_finite() {
    let shapes_: Vec<PathData> = vec![
        PathData::new(vec![]),
        shapes::rectangle(Rect::new(0.0, 0.0, 0.0, 0.0)),
        shapes::rectangle(Rect::new(0.0, 0.0, 1e-9, 50.0)),
        shapes::rectangle(Rect::new(-1e7, -1e7, -1e7 + 30.0, -1e7 + 30.0)),
        PathData::new(vec![vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(f64::NAN, 3.0), Point::new(9.0, 9.0)], true)]),
        PathData::new(vec![vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(50.0, 50.0)], false)]),
        shapes::ellipse(Rect::new(0.0, 0.0, 40.0, 40.0)),
    ];
    let rows: Vec<ScreenRow> = [
        Screen { lpi: f64::NAN, angle: f64::INFINITY, quality: u32::MAX, amount: f64::NAN, ..Screen::default() },
        Screen { lpi: -3.0, amount: -1e9, tone: ToneSource::Fixed, ..Screen::default() },
        Screen { lpi: 1e300, quality: 0, tone: ToneSource::Fixed, amount: 1e300, invert: true, ..Screen::default() },
        Screen { spot: SpotShape::Cross, tone: ToneSource::Image, ..Screen::default() },
    ]
    .into_iter()
    .map(|params| ScreenRow { params, image: None })
    .collect();
    for p in &shapes_ {
        for row in &rows {
            for img in [None, Some(Ok(half_black(Rect::new(0.0, 0.0, 40.0, 40.0)))), Some(Err("gone".to_string()))] {
                let (m, _) = screened_with(&node(p.clone(), black()), ScreenRow { image: img, ..row.clone() });
                if let NodeKind::Path { path, .. } = &m.kind {
                    assert!(
                        path.subpaths
                            .iter()
                            .flat_map(|s| &s.anchors)
                            .all(|a| a.p.is_finite() || p.subpaths.iter().flat_map(|s| &s.anchors).any(|b| !b.p.is_finite()))
                    );
                }
            }
        }
    }
}
