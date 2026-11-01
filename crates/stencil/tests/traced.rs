//! The stencil of a real traced design (`fixtures/traced.svg`: three flat colours, 920 × 752,
//! 85 jagged paths plus tracer specks), and timings (`--ignored`, release).
//!
//! The fixture is the owner's file and is **not in this repository**: its markup looks like another
//! application's SVG export, and the clean-room rule (CLAUDE.md) keeps such files out of the repo
//! (used locally, never committed; `tests/fixtures/.gitignore` ignores it). These tests read it at
//! run time from `$PICABIA_TRACED_SVG`, else from `tests/fixtures/traced.svg`, and pass with a note
//! when it is absent. `tests/generated.rs` runs the same checks on a design traced by VectorCraft's
//! own Image Trace from a generated image, always.
//!
//! The SVG is read by a minimal test-only parser (class fills from the `<style>` block; `path`,
//! `rect` and `circle` in document order; `translate`/`rotate` transforms), enough for this file.
//! `figcraft-svg` (L4) would do as a dev-dependency (`cargo xtask layers` skips dev-deps) but pulls
//! in text and fonts for no gain here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/fixture.rs"]
mod fixture;

use std::collections::HashMap;
use std::time::Instant;

use vectorcraft_geom::kurbo::{Affine, BezPath, Circle, Point, Rect, Shape, Vec2};
use vectorcraft_modifiers::{Repeat, Stencil, StencilLimitMode, StencilTieMode};
use vectorcraft_stencil::ops::split_subpaths;
use vectorcraft_stencil::{LaserInput, LaserParams, Paint, compute, compute_repeat, laser_job, laser_svg, mm_per_unit, test_card, visible_regions};

/// The owner's traced design, when it is on this machine.
fn svg() -> Option<String> {
    let path = std::env::var_os("PICABIA_TRACED_SVG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/traced.svg"));
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!("skipped: {} isn't here (the owner's local fixture; see the module docs)", path.display());
            None
        }
    }
}
const BOUNDS: Rect = Rect::new(0.0, 0.0, 920.0, 752.0);

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let len = tag.get(start..)?.find('"')?;
    tag.get(start..start + len)
}

fn num(tag: &str, name: &str) -> f64 {
    attr(tag, name).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
}

/// `translate(x y) rotate(deg)` sequences, applied left to right as SVG composes them.
fn transform(s: &str) -> Affine {
    let mut xf = Affine::IDENTITY;
    for part in s.split(')') {
        let Some((name, args)) = part.split_once('(') else { continue };
        let a: Vec<f64> = args.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect();
        let op = match name.trim() {
            "translate" => Affine::translate((a[0], a.get(1).copied().unwrap_or(0.0))),
            "rotate" => Affine::rotate(a[0].to_radians()),
            other => panic!("transform {other} not handled"),
        };
        xf *= op;
    }
    xf
}

/// `.cls-N { fill: #rrggbb; }` from the style block.
fn class_fills(svg: &str) -> HashMap<String, [f32; 4]> {
    let mut out = HashMap::new();
    let mut rest = svg;
    while let Some(i) = rest.find(".cls-") {
        rest = &rest[i + 1..];
        let name: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '{').collect();
        let body = &rest[rest.find('{').unwrap()..rest.find('}').unwrap()];
        let hex = body.split("fill:").nth(1).unwrap().trim().trim_end_matches(';').trim();
        let c = vectorcraft_modifiers::parse_hex_colour(hex).unwrap();
        out.insert(name, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]);
    }
    out
}

/// Every filled element of the fixture as a paint, bottom first (`None`: the fixture isn't here).
fn traced() -> Option<Vec<Paint>> {
    let svg = svg()?;
    let fills = class_fills(&svg);
    let mut paints = Vec::new();
    for chunk in svg.split('<').skip(1) {
        let tag = chunk.split('>').next().unwrap();
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap();
        let path = match name {
            "path" => BezPath::from_svg(attr(tag, "d").unwrap()).unwrap(),
            "rect" => Rect::new(num(tag, "x"), num(tag, "y"), num(tag, "x") + num(tag, "width"), num(tag, "y") + num(tag, "height")).to_path(0.01),
            "circle" => Circle::new((num(tag, "cx"), num(tag, "cy")), num(tag, "r")).to_path(0.01),
            _ => continue,
        };
        let xf = attr(tag, "transform").map(transform).unwrap_or(Affine::IDENTITY);
        let colour = fills[attr(tag, "class").unwrap()];
        paints.push(Paint { path: xf * path, colour, order: paints.len() as u32 });
    }
    Some(paints)
}

#[test]
fn the_fixture_parses() {
    let Some(paints) = traced() else { return };
    assert_eq!(paints.len(), 100, "85 paths, 12 rects, 3 circles");
    let bb = paints.iter().fold(Rect::ZERO, |b, p| b.union(p.path.bounding_box()));
    assert!((bb.width() - 920.0).abs() < 1.0 && (bb.height() - 752.0).abs() < 1.0, "{bb:?}");
    let v = visible_regions(&paints, 0.04, 0.0);
    assert_eq!(v.regions.len(), 3);
    let total: f64 = v.regions.iter().map(|r| r.area).sum();
    assert!(total > BOUNDS.area() * 0.95 && total < BOUNDS.area() * 1.001, "{total}");
}

#[test]
fn the_traced_design_makes_a_sane_stencil() {
    let Some(paints) = traced() else { return };
    let p = Stencil::default();
    let out = compute(&paints, &p, BOUNDS).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(!out.cut.elements().is_empty() && !out.paper.elements().is_empty());
    let (sheet, cut, paper) = (out.sheet.area(), out.cut.area(), out.paper.area());
    assert!(cut > 0.01 * sheet && paper > 0.3 * sheet, "cut {cut}, paper {paper}, sheet {sheet}");
    assert!((paper - (sheet - cut)).abs() < sheet * 1e-9);
    // Specks gone: nothing below the area limit is left.
    let mpu = mm_per_unit(p.width_mm, BOUNDS).unwrap();
    let min = p.min_cut_area_mm2 / (mpu * mpu);
    assert!(out.stats.dropped_specks > 0, "{:?}", out.stats);
    for s in split_subpaths(&out.cut) {
        assert!(s.area().abs() >= min * 0.8, "contour of {} < {min}", s.area());
    }
    let sw: Vec<(&str, bool)> = out.colours.iter().map(|c| (c.hex.as_str(), c.cut)).collect();
    assert_eq!(sw.len(), 3, "{sw:?}");
    assert!(sw.contains(&("#e7cb5b", true)) && sw.contains(&("#cbab9e", false)) && sw.contains(&("#28262e", false)), "{sw:?}");
    // The cut is the yellow, cleaned: close to its visible area.
    let yellow = out.colours.iter().find(|c| c.cut).unwrap().area;
    assert!((out.stats.area_cut_mm2 - yellow).abs() < yellow * 0.05, "{} vs {yellow}", out.stats.area_cut_mm2);
    // Inverted: the two darker colours are cut instead.
    let inv = compute(&paints, &Stencil { invert: true, ..p }, BOUNDS).unwrap();
    assert!(inv.cut.area() > cut && inv.stats.cut_pieces > 0);
}

/// Islands still floating in a cut: holes in it (contours nested at odd depth), counted by
/// brute-force containment, independent of the crate's own tree.
fn floating(cut: &BezPath) -> usize {
    let subs: Vec<BezPath> = split_subpaths(cut);
    let probe = |s: &BezPath| {
        let mut best = (0.0, Point::ZERO);
        let mut last: Option<Point> = None;
        for el in s.elements() {
            let p = match el {
                vectorcraft_geom::kurbo::PathEl::MoveTo(p) | vectorcraft_geom::kurbo::PathEl::LineTo(p) => *p,
                _ => continue,
            };
            if let Some(l) = last {
                let d: f64 = (p - l).hypot();
                if d > best.0 {
                    best = (d, l.midpoint(p));
                }
            }
            last = Some(p);
        }
        best.1
    };
    let areas: Vec<f64> = subs.iter().map(|s| s.area().abs()).collect();
    (0..subs.len())
        .filter(|&i| {
            let p = probe(&subs[i]);
            let depth = (0..subs.len()).filter(|&j| j != i && areas[j] > areas[i] && subs[j].winding(p) != 0).count();
            depth % 2 == 1
        })
        .count()
}

#[test]
fn the_traced_design_has_no_floating_islands() {
    let Some(paints) = traced() else { return };
    let gauze = compute(&paints, &Stencil { tie_mode: StencilTieMode::Gauze, ..Stencil::default() }, BOUNDS).unwrap();
    let found = gauze.stats.islands;
    assert!(found > 0, "the traced art has paper islands (the crescent in the swoosh): {:?}", gauze.stats);
    assert_eq!((gauze.stats.untied, floating(&gauze.cut)), (found, found), "gauze leaves them");
    for mode in [StencilTieMode::Ties, StencilTieMode::Pattern] {
        let out = compute(&paints, &Stencil { tie_mode: mode, ..Stencil::default() }, BOUNDS).unwrap();
        assert_eq!(out.stats.islands, found, "{mode:?}");
        assert_eq!(out.stats.untied, 0, "{mode:?}: {:?} {:?}", out.stats, out.warnings);
        assert_eq!(floating(&out.cut), 0, "{mode:?}: nothing floats");
        assert!(out.islands.iter().all(|i| i.tied > 0 || i.pattern_held), "{mode:?}: {:?}", out.islands);
        let sheet = out.sheet.area();
        assert!((out.paper.area() - (sheet - out.cut.area())).abs() < sheet * 1e-9);
        eprintln!("{mode:?}: {:?}", out.stats);
    }
}

fn time<T>(label: &str, runs: u32, mut f: impl FnMut() -> T) -> T {
    let mut out = f();
    let t = Instant::now();
    for _ in 0..runs {
        out = f();
    }
    eprintln!("{label}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs));
    out
}

fn report(name: &str, paints: &[Paint], bounds: Rect) {
    let p = Stencil::default();
    let mpu = mm_per_unit(p.width_mm, bounds).unwrap();
    let elements: usize = paints.iter().map(|p| p.path.elements().len()).sum();
    eprintln!("{name}: {} paints, {elements} elements", paints.len());
    let tol = p.tolerance_mm / mpu;
    time(&format!("  {name} visible regions"), 5, || visible_regions(paints, tol / 4.0, tol / 2.0));
    let raw = Stencil { min_cut_width_mm: 0.0, min_bridge_mm: 0.0, ..p.clone() };
    time(&format!("  {name} compute, no opening/closing"), 5, || compute(paints, &raw, bounds).unwrap());
    let gauze = Stencil { tie_mode: StencilTieMode::Gauze, ..p.clone() };
    time(&format!("  {name} compute, gauze (islands found, no ties)"), 5, || compute(paints, &gauze, bounds).unwrap());
    let pattern = Stencil { tie_mode: StencilTieMode::Pattern, ..p.clone() };
    time(&format!("  {name} compute, pattern"), 5, || compute(paints, &pattern, bounds).unwrap());
    let flag = Stencil { limit_mode: StencilLimitMode::Flag, ..p.clone() };
    let f = time(&format!("  {name} compute, limits flagged"), 5, || compute(paints, &flag, bounds).unwrap());
    eprintln!("  {name} flagged problems: {:?}", f.stats.problems_by_kind);
    let out = time(&format!("  {name} compute, defaults (ties)"), 5, || compute(paints, &p, bounds).unwrap());
    eprintln!("  {name} out: {} cut elements, {:?}", out.cut.elements().len(), out.stats);
}

#[test]
#[ignore = "timing: cargo test --release -p vectorcraft-stencil --test traced -- --ignored --nocapture"]
fn timing() {
    let Some(one) = traced() else { return };
    report("traced.svg", &one, BOUNDS);
    // A 3 × 3 grid of it, judged on the repeat (the block is the whole design here: the worst case).
    let mut grid = Vec::new();
    for i in 0..3 {
        for j in 0..3 {
            let base = grid.len() as u32;
            let v = Vec2::new(BOUNDS.width() * f64::from(i), BOUNDS.height() * f64::from(j));
            grid.extend(one.iter().map(|p| Paint { path: Affine::translate(v) * p.path.clone(), colour: p.colour, order: base + p.order }));
        }
    }
    let gb = Rect::new(0.0, 0.0, BOUNDS.width() * 3.0, BOUNDS.height() * 3.0);
    let rep = Repeat { origin: Point::ZERO, a: Vec2::new(BOUNDS.width(), 0.0), b: Vec2::new(0.0, BOUNDS.height()) };
    let p = Stencil { width_mm: 900.0, ..Stencil::default() };
    time("traced.svg 3×3 grid, sheet", 2, || compute(&grid, &p, gb).unwrap());
    let out = time("traced.svg 3×3 grid, repeat-aware", 2, || compute_repeat(&grid, &p, gb, Some(rep)).unwrap());
    eprintln!("  grid out: {:?} {:?}", out.stats, out.warnings);
    let (paints, _) = fixture::design(300, 42);
    report("synthetic 300", &paints, fixture::BOUNDS);
}

/// The traced design's stencil as a laser job input (the engine builds the same).
fn laser_input(p: &Stencil) -> Option<LaserInput> {
    let out = compute(&traced()?, p, BOUNDS).unwrap();
    Some(LaserInput {
        cut: out.cut,
        sheet: out.sheet,
        mm_per_unit: mm_per_unit(p.width_mm, BOUNDS).unwrap(),
        hoshi: out.hoshi,
        hoshi_d_mm: p.hoshi_diameter_mm,
        label_lines: vec!["traced".into(), "Sheet 330 × 275 mm".to_string(), "paper".into()],
        repeat_cell: None,
    })
}

#[test]
fn the_traced_stencil_exports_for_the_laser() {
    let p = Stencil::default();
    let Some(input) = laser_input(&p) else { return };
    let job = laser_job(&input, &LaserParams::default());
    assert_eq!(job.sheets.len(), 1, "330 mm sheet fits a 600 × 400 bed: {:?}", job.warnings);
    let s = &job.sheets[0];
    assert_eq!(s.loose_pieces, 0, "ties hold every island: {:?}", job.warnings);
    assert!(!s.engrave.is_empty(), "labelled: {:?}", job.warnings);
    assert!(s.cut.len() > 10);
    let svg = laser_svg(s, &LaserParams::default());
    assert!(svg.contains(&format!("width=\"{}mm\"", (s.w_mm * 1000.0).round() / 1000.0)));
    // Split on a small bed: every piece fits it.
    let small = LaserParams { bed_w_mm: 200.0, bed_h_mm: 200.0, ..LaserParams::default() };
    let job = laser_job(&input, &small);
    assert!(job.sheets.len() >= 4, "{:?}", job.warnings);
    assert!(job.sheets.iter().all(|s| s.w_mm <= 200.0 && s.h_mm <= 200.0));
    // Optional look: LASER_OUT=dir writes the SVGs.
    if let Ok(dir) = std::env::var("LASER_OUT") {
        let dir = std::path::Path::new(&dir);
        std::fs::write(dir.join("traced.svg"), laser_svg(&laser_job(&input, &LaserParams::default()).sheets[0], &LaserParams::default())).unwrap();
        for (i, s) in job.sheets.iter().enumerate() {
            std::fs::write(dir.join(format!("traced-split-{}.svg", i + 1)), laser_svg(s, &small)).unwrap();
        }
        let card = test_card(&vectorcraft_modifiers::StencilMaterial::Paper.limits().unwrap(), "Paper", 0.1);
        std::fs::write(dir.join("card.svg"), laser_svg(&card, &LaserParams::default())).unwrap();
    }
}

#[test]
#[ignore = "timing: cargo test --release -p vectorcraft-stencil --test traced laser_timing -- --ignored --nocapture"]
fn laser_timing() {
    let Some(input) = laser_input(&Stencil::default()) else { return };
    let elements = input.cut.elements().len();
    let job = time(&format!("traced.svg laser_job, one sheet ({elements} cut elements)"), 10, || laser_job(&input, &LaserParams::default()));
    let points: usize = job.sheets.iter().flat_map(|s| s.cut.iter()).map(Vec::len).sum();
    eprintln!("  {} contours, {points} points, warnings {:?}", job.sheets[0].cut.len(), job.warnings);
    time("traced.svg laser_job, kerf 0", 10, || laser_job(&input, &LaserParams { kerf_mm: 0.0, ..LaserParams::default() }));
    let small = LaserParams { bed_w_mm: 200.0, bed_h_mm: 200.0, ..LaserParams::default() };
    let job = time("traced.svg laser_job, 200 × 200 bed", 5, || laser_job(&input, &small));
    eprintln!("  {} sheets", job.sheets.len());
    let card = vectorcraft_modifiers::StencilMaterial::Paper.limits().unwrap();
    time("test card", 10, || test_card(&card, "Paper", 0.1));
    let s = &laser_job(&input, &LaserParams::default()).sheets[0];
    time("laser_svg", 10, || laser_svg(s, &LaserParams::default()));
}
