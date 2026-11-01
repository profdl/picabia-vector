//! Every stage on the synthetic traced-style fixture.

use crate::ops::BoolOp;
use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilTieMode};

use crate::fixture::{self, BOUNDS};
use crate::{Selector, StencilOut, compute, mm_per_unit, rgb8, visible_regions};

fn cut_within(out: &StencilOut, r: Rect) -> f64 {
    crate::ops::boolean_step(&out.cut, &r.inflate(0.5, 0.5).to_path(0.1), BoolOp::Intersect).unwrap().area()
}

#[test]
fn the_fixture_is_deterministic_and_has_every_case() {
    let (a, c) = fixture::design(120, 7);
    let (b, _) = fixture::design(120, 7);
    assert_eq!(a, b);
    assert_ne!(a, fixture::design(120, 8).0);
    assert!(c.blobs > 50 && c.specks > 10 && c.light_specks > 4 && c.nests >= 1, "{c:?}");
    assert!(!c.glyphs.is_empty() && !c.tiny_glyphs.is_empty());
    assert!(a.len() >= 120);
}

#[test]
fn visible_regions_tile_the_design() {
    let (paints, _) = fixture::design(120, 1);
    let v = visible_regions(&paints, 0.01, 0.05);
    assert!(v.warnings.is_empty(), "{:?}", v.warnings);
    assert_eq!(v.regions.len(), 4, "dark, mid, light, white");
    assert!(v.regions.iter().all(|r| r.area > 0.0));
    let total: f64 = v.regions.iter().map(|r| r.area).sum();
    let want = BOUNDS.area();
    assert!((total - want).abs() < want * 1e-3, "the dark ground underlies everything: {total} vs {want}");
    // The ground is listed first (bottom first).
    assert_eq!(v.regions[0].colour, rgb8(fixture::DARK).unwrap());
}

#[test]
fn colours_split_by_luminance() {
    let s = Selector::new(&Stencil::default());
    let cut = |c| s.cuts(rgb8(c).unwrap());
    assert!(!cut(fixture::DARK) && !cut(fixture::MID));
    assert!(cut(fixture::LIGHT) && cut(fixture::WHITE));
}

#[test]
fn the_stencil_of_the_fixture_is_clean() {
    let (paints, c) = fixture::design(120, 3);
    // Gauze: the cut before ties change it (the nest's disc stays an island).
    let p = Stencil { tie_mode: StencilTieMode::Gauze, ..Stencil::default() };
    let out = compute(&paints, &p, BOUNDS).unwrap();
    assert!(out.warnings.iter().all(|w| w.contains("gauze")), "{:?}", out.warnings);
    assert!(out.stats.islands >= 1 && out.stats.untied == out.stats.islands, "{:?}", out.stats);
    let mpu = mm_per_unit(p.width_mm, BOUNDS).unwrap();
    assert_eq!(out.mm_per_unit, mpu);
    assert!((out.sheet.width() * mpu - (300.0 + 30.0)).abs() < 1e-9, "300 mm wide plus 15 mm a side");
    assert!(!out.cut.elements().is_empty() && !out.paper.elements().is_empty());
    let (sheet, cut, paper) = (out.sheet.area(), out.cut.area(), out.paper.area());
    assert!(cut > 0.0 && (paper - (sheet - cut)).abs() < sheet * 1e-9);
    // No cut piece below the minimum area survives (simplification may shave a little).
    let min = p.min_cut_area_mm2 / (mpu * mpu);
    let contours = split_subpaths(&out.cut);
    for s in &contours {
        assert!(s.area().abs() >= min * 0.8, "a contour of {} < {min}", s.area());
    }
    assert!(out.stats.dropped_specks > 0, "{:?}", out.stats);
    assert!(contours.iter().any(|s| s.area() < 0.0), "the nest's dark disc is a paper hole in the cut");
    assert_eq!(out.stats.cut_pieces, contours.iter().filter(|s| s.area() > 0.0).count());
    assert!(out.stats.cut_length_mm > 0.0 && (out.stats.area_cut_mm2 - cut * mpu * mpu).abs() < 1e-6);
    // Readable glyphs are cut, glyphs with bars thinner than the minimum cut width are not.
    for g in &c.glyphs {
        assert!(cut_within(&out, *g) > 1.0, "glyph at {g:?} kept");
    }
    for g in &c.tiny_glyphs {
        assert!(cut_within(&out, *g) < 0.05, "tiny glyph at {g:?} gone: {}", cut_within(&out, *g));
    }
    let sw: Vec<(&str, bool)> = out.colours.iter().map(|c| (c.hex.as_str(), c.cut)).collect();
    assert_eq!(sw.len(), 4);
    assert!(sw.contains(&("#e7cb5b", true)) && sw.contains(&("#28262e", false)), "{sw:?}");
}

#[test]
fn inverting_cuts_the_rest() {
    let (paints, _) = fixture::design(90, 5);
    let raw = Stencil { min_cut_area_mm2: 0.0, min_cut_width_mm: 0.0, min_bridge_mm: 0.0, tie_mode: StencilTieMode::Gauze, ..Stencil::default() };
    let a = compute(&paints, &raw, BOUNDS).unwrap();
    let b = compute(&paints, &Stencil { invert: true, ..raw }, BOUNDS).unwrap();
    let total = a.cut.area() + b.cut.area();
    assert!((total - BOUNDS.area()).abs() < BOUNDS.area() * 2e-3, "{total}");
    let overlap = crate::ops::boolean_step(&a.cut, &b.cut, BoolOp::Intersect).unwrap().area();
    assert!(overlap < BOUNDS.area() * 2e-3, "{overlap}");
}

#[test]
fn the_cleanup_steps_each_remove_what_they_should() {
    let (paints, _) = fixture::design(90, 11);
    let none = Stencil { min_cut_area_mm2: 0.0, min_cut_width_mm: 0.0, min_bridge_mm: 0.0, ..Stencil::default() };
    let pieces = |p: &Stencil| compute(&paints, p, BOUNDS).unwrap().stats;
    let base = pieces(&none);
    let area = pieces(&Stencil { min_cut_area_mm2: 0.5, ..none.clone() });
    assert!(area.cut_pieces < base.cut_pieces && area.dropped_specks > 0, "{base:?} → {area:?}");
    let open = pieces(&Stencil { min_cut_width_mm: 0.3, ..none.clone() });
    assert!(open.cut_pieces < base.cut_pieces, "specks and thin glyphs open away: {base:?} → {open:?}");
    let close = pieces(&Stencil { min_bridge_mm: 3.0, ..none.clone() });
    assert!(close.cut_pieces < base.cut_pieces, "nearby cuts join: {base:?} → {close:?}");
}

#[test]
fn the_fixture_ties_every_island() {
    for seed in [3, 8, 21] {
        let (paints, _) = fixture::design(120, seed);
        for mode in [StencilTieMode::Ties, StencilTieMode::Pattern] {
            let out = compute(&paints, &Stencil { tie_mode: mode, ..Stencil::default() }, BOUNDS).unwrap();
            assert_eq!(out.stats.untied, 0, "seed {seed} {mode:?}: {:?} {:?}", out.stats, out.warnings);
            assert!(out.islands.iter().all(|i| i.tied > 0 || i.pattern_held), "seed {seed} {mode:?}");
            assert_eq!(crate::islands::Tree::new(&out.cut).unwrap().islands().len(), 0);
        }
    }
}
