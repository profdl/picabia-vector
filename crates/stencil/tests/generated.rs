//! The trace → stencil path end to end, on a design traced by VectorCraft's own Image Trace from a
//! generated image (three flat colours with jagged edges, a ring that leaves a paper island, and
//! one-pixel specks), built as the Image Trace command builds its result (a group of filled
//! paths in the tracer's order) and gathered with [`gather`]. The same checks as `tests/traced.rs`
//! makes on the owner's traced design, which isn't in the repository.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{Color, Paint as Fill};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilTieMode};
use vectorcraft_stencil::ops::split_subpaths;
use vectorcraft_stencil::{LaserInput, LaserParams, Paint, compute, gather, laser_job, laser_svg, luminance, mm_per_unit, visible_regions};

const W: u32 = 460;
const H: u32 = 376;
const BOUNDS: Rect = Rect::new(0.0, 0.0, W as f64, H as f64);
const DARK: [u8; 3] = [0x28, 0x26, 0x2e];
const BEIGE: [u8; 3] = [0xcb, 0xab, 0x9e];
const YELLOW: [u8; 3] = [0xe7, 0xcb, 0x5b];

/// `#rrggbb` as bytes.
fn rgb(hex: &str) -> [u8; 3] {
    let c = vectorcraft_modifiers::parse_hex_colour(hex).unwrap();
    [c[0], c[1], c[2]]
}

/// A deterministic hash of a pixel to 0..1.
fn noise(x: u32, y: u32) -> f64 {
    let h = u64::from(x).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(y).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    let h = (h ^ (h >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The generated image: a dark ground with a beige band along the bottom, a jagged yellow ring
/// (its inside is a paper island once yellow is cut), a jagged yellow blob with a beige notch, and
/// one-pixel yellow specks on the ground.
fn image() -> Vec<u8> {
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let jag = |cx: f64, cy: f64, r: f64| {
                let a = (fy - cy).atan2(fx - cx);
                r + 4.0 * (7.0 * a).sin() + 2.0 * (13.0 * a + 1.0).sin()
            };
            let d1 = (fx - 140.0).hypot(fy - 140.0);
            let d2 = (fx - 340.0).hypot(fy - 130.0);
            let c = if d1 < jag(140.0, 140.0, 85.0) && d1 > jag(140.0, 140.0, 45.0) {
                YELLOW
            } else if d2 < jag(340.0, 130.0, 60.0) {
                if (fx - 365.0).hypot(fy - 120.0) < 18.0 { BEIGE } else { YELLOW }
            } else if fy > 270.0 + 6.0 * (fx / 23.0).sin() {
                BEIGE
            } else if x % 41 == 7 && y % 37 == 11 && noise(x, y) < 0.8 {
                YELLOW
            } else {
                DARK
            };
            rgba.extend([c[0], c[1], c[2], 255]);
        }
    }
    rgba
}

/// The image traced ("3 Colors", noise 1 so the specks stay) and gathered: bottom first.
fn traced() -> Vec<Paint> {
    let raster = vectorcraft_trace::Raster::new(W, H, image()).unwrap();
    let params = vectorcraft_trace::TraceParams { noise: 1, ..vectorcraft_trace::preset("3 colors").unwrap() };
    let res = vectorcraft_trace::trace(&raster, &params);
    let kids: Vec<Arc<Node>> = res
        .paths
        .iter()
        .enumerate()
        .map(|(i, tp)| {
            let fill = Fill::solid(Color::rgb8(tp.color[0], tp.color[1], tp.color[2]));
            let ap = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(fill))], ..Appearance::default() };
            Arc::new(Node::path(NodeId(10 + i as u64), tp.path.clone(), ap))
        })
        .collect();
    let group = Node::group(NodeId(1), kids);
    let g = gather(&Document::new(612.0, 792.0), &group, &[], None);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    assert_eq!(g.paints.len(), res.paths.len());
    g.paints
}

#[test]
fn the_traced_image_gathers_three_colours() {
    let paints = traced();
    assert!(paints.len() > 10, "{} paths", paints.len());
    let v = visible_regions(&paints, 0.04, 0.0);
    assert_eq!(v.regions.len(), 3, "{:?}", v.regions.iter().map(|r| r.colour).collect::<Vec<_>>());
    let total: f64 = v.regions.iter().map(|r| r.area).sum();
    assert!(total > BOUNDS.area() * 0.95 && total < BOUNDS.area() * 1.001, "{total}");
}

#[test]
fn the_traced_image_makes_a_sane_stencil() {
    let paints = traced();
    let p = Stencil::default();
    let out = compute(&paints, &p, BOUNDS).unwrap();
    assert!(!out.cut.elements().is_empty() && !out.paper.elements().is_empty());
    let (sheet, cut, paper) = (out.sheet.area(), out.cut.area(), out.paper.area());
    assert!(cut > 0.01 * sheet && paper > 0.3 * sheet, "cut {cut}, paper {paper}, sheet {sheet}");
    assert!((paper - (sheet - cut)).abs() < sheet * 1e-9);
    // The specks are gone: nothing below the area limit is left.
    let mpu = mm_per_unit(p.width_mm, BOUNDS).unwrap();
    let min = p.min_cut_area_mm2 / (mpu * mpu);
    assert!(out.stats.dropped_specks > 0, "{:?}", out.stats);
    for s in split_subpaths(&out.cut) {
        assert!(s.area().abs() >= min * 0.8, "contour of {} < {min}", s.area());
    }
    // Only the yellow (the one colour lighter than the threshold) is cut.
    assert_eq!(out.colours.len(), 3);
    let cut_colours: Vec<_> = out.colours.iter().filter(|c| c.cut).collect();
    assert_eq!(cut_colours.len(), 1, "{:?}", out.colours);
    assert!(luminance(rgb(&cut_colours[0].hex)) > 0.5 && out.colours.iter().filter(|c| !c.cut).all(|c| luminance(rgb(&c.hex)) < 0.5));
    let yellow = cut_colours[0].area;
    assert!((out.stats.area_cut_mm2 - yellow).abs() < yellow * 0.05, "{} vs {yellow}", out.stats.area_cut_mm2);
    // Inverted: the two darker colours are cut instead.
    let inv = compute(&paints, &Stencil { invert: true, ..p }, BOUNDS).unwrap();
    assert!(inv.cut.area() > cut && inv.stats.cut_pieces > 0);
}

/// Islands still floating in a cut: holes in it (contours nested at odd depth), counted by
/// brute-force containment.
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
fn the_ring_island_is_held() {
    let paints = traced();
    let gauze = compute(&paints, &Stencil { tie_mode: StencilTieMode::Gauze, ..Stencil::default() }, BOUNDS).unwrap();
    let found = gauze.stats.islands;
    assert!(found > 0, "the ring leaves a paper island: {:?}", gauze.stats);
    assert_eq!((gauze.stats.untied, floating(&gauze.cut)), (found, found), "gauze leaves them");
    for mode in [StencilTieMode::Ties, StencilTieMode::Pattern] {
        let out = compute(&paints, &Stencil { tie_mode: mode, ..Stencil::default() }, BOUNDS).unwrap();
        assert_eq!(out.stats.islands, found, "{mode:?}");
        assert_eq!(out.stats.untied, 0, "{mode:?}: {:?} {:?}", out.stats, out.warnings);
        assert_eq!(floating(&out.cut), 0, "{mode:?}: nothing floats");
        assert!(out.islands.iter().all(|i| i.tied > 0 || i.pattern_held), "{mode:?}: {:?}", out.islands);
    }
}

#[test]
fn the_traced_stencil_exports_for_the_laser() {
    let p = Stencil::default();
    let out = compute(&traced(), &p, BOUNDS).unwrap();
    let input = LaserInput {
        cut: out.cut,
        sheet: out.sheet,
        mm_per_unit: mm_per_unit(p.width_mm, BOUNDS).unwrap(),
        hoshi: out.hoshi,
        hoshi_d_mm: p.hoshi_diameter_mm,
        label_lines: vec!["generated".into(), "paper".into()],
        repeat_cell: None,
    };
    let job = laser_job(&input, &LaserParams::default());
    assert_eq!(job.sheets.len(), 1, "a 330 mm sheet fits a 600 × 400 bed: {:?}", job.warnings);
    let s = &job.sheets[0];
    assert_eq!(s.loose_pieces, 0, "ties hold every island: {:?}", job.warnings);
    assert!(!s.engrave.is_empty(), "labelled: {:?}", job.warnings);
    assert!(s.cut.len() > 2);
    let svg = laser_svg(s, &LaserParams::default());
    assert!(svg.contains(&format!("width=\"{}mm\"", (s.w_mm * 1000.0).round() / 1000.0)));
    let small = LaserParams { bed_w_mm: 200.0, bed_h_mm: 200.0, ..LaserParams::default() };
    let job = laser_job(&input, &small);
    assert!(job.sheets.len() >= 4, "{:?}", job.warnings);
    assert!(job.sheets.iter().all(|s| s.w_mm <= 200.0 && s.h_mm <= 200.0));
}
