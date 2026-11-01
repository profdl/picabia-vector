//! The laser test card.

use vectorcraft_geom::kurbo::{Point, Rect};
use vectorcraft_modifiers::{MaterialLimits, StencilMaterial};

use crate::laser::bounds;
use crate::{BRIDGES_MM, CARD_H_MM, CARD_W_MM, CORNERS_DEG, HOLES_MM, LaserParams, LaserSheet, SLITS_MM, laser_svg, test_card};

fn paper() -> MaterialLimits {
    StencilMaterial::Paper.limits().unwrap()
}

fn rects(s: &LaserSheet) -> Vec<Rect> {
    s.cut.iter().map(|c| bounds(c)).collect()
}

/// Engraved polylines lying within the band `y0..y1`.
fn engraved_in(s: &LaserSheet, y0: f64, y1: f64) -> usize {
    s.engrave.iter().filter(|l| l.iter().all(|p| p.y >= y0 && p.y <= y1)).count()
}

#[test]
fn the_card_has_every_row_and_fits() {
    let s = test_card(&paper(), "Paper", 0.0);
    assert_eq!((s.w_mm, s.h_mm), (CARD_W_MM, CARD_H_MM));
    let b = rects(&s);
    let want = 2 * BRIDGES_MM.len() + HOLES_MM.len() + SLITS_MM.len() + CORNERS_DEG.len() + 1 + 5 + 1;
    assert_eq!(b.len(), want, "bridges, holes, slits, corners, gauge square, comb, outline");
    let outline = *b.last().unwrap();
    assert!((outline.width() - CARD_W_MM).abs() < 1e-6 && (outline.height() - CARD_H_MM).abs() < 1e-6, "{outline:?}");
    let inner = Rect::new(3.0, 3.0, CARD_W_MM - 3.0, CARD_H_MM - 3.0);
    for r in &b[..b.len() - 1] {
        assert!(inner.contains(Point::new(r.x0, r.y0)) && inner.contains(Point::new(r.x1, r.y1)), "inside the margin: {r:?}");
    }
    for p in s.engrave.iter().flatten() {
        assert!(inner.contains(*p), "engraving inside the margin: {p:?}");
    }
    // No feature meets another (or the text): bounds pairwise apart.
    for (i, a) in b[..b.len() - 1].iter().enumerate() {
        for c in &b[i + 1..b.len() - 1] {
            assert!(a.intersect(*c).area() <= 0.0, "{a:?} meets {c:?}");
        }
    }
    // The gauge square, exact.
    assert!(b.iter().any(|r| (r.width() - 20.0).abs() < 1e-6 && (r.height() - 20.0).abs() < 1e-6));
    // Every row is labelled: header, bridges, holes, slits, corners, footer.
    for (y0, y1) in [(3.0, 7.5), (11.0, 22.0), (24.0, 31.0), (34.0, 46.0), (49.0, 59.0), (62.0, 70.0)] {
        assert!(engraved_in(&s, y0, y1) > 0, "text in {y0}..{y1}");
    }
    // Paper marks: bridge 0.5 and tie 1.0 (two triangles), cut width 0.3 for holes and slits,
    // corner 30°: five triangles (closed 4-point polylines 1 mm wide).
    let marks = s.engrave.iter().filter(|l| l.len() == 4 && l.first() == l.last() && (bounds(l).width() - 1.0).abs() < 1e-9).count();
    assert_eq!(marks, 5);
    assert_eq!(s.name, "test card");
    assert!(laser_svg(&s, &LaserParams::default()).contains(r#"width="120mm" height="80mm""#));
}

#[test]
fn the_card_is_kerf_compensated() {
    let k = 0.1;
    let s = test_card(&paper(), "Paper", k);
    assert!((s.w_mm - 120.1).abs() < 1e-9);
    let b = rects(&s);
    assert!(b.iter().any(|r| (r.width() - 19.9).abs() < 2e-3 && (r.height() - 19.9).abs() < 2e-3), "the gauge's path is 20 − kerf");
    // The 0.1 mm slit is no wider than the kerf: gone.
    let full = test_card(&paper(), "Paper", 0.0);
    assert_eq!(b.len(), rects(&full).len() - 1);
    let footer = engraved_text_count(&s);
    assert!(footer > 0);
    // Junk limits and kerf don't break it.
    let junk =
        MaterialLimits { min_cut_width_mm: f64::NAN, min_bridge_mm: -1.0, tie_width_mm: f64::INFINITY, min_corner_angle: 1e9, kerf_mm: f64::NAN };
    let s = test_card(&junk, &"x".repeat(1000), f64::NAN);
    assert!(s.cut.len() > 40 && s.w_mm.is_finite());
    let s = test_card(&paper(), "", 99.0);
    assert!((s.w_mm - 122.0).abs() < 1e-9, "kerf clamped to 2");
}

fn engraved_text_count(s: &LaserSheet) -> usize {
    s.engrave.len()
}

#[test]
fn marks_follow_the_material() {
    let film = test_card(&StencilMaterial::Film.limits().unwrap(), "Film", 0.08);
    let card = test_card(&StencilMaterial::Card.limits().unwrap(), "Card", 0.15);
    let mark_x = |s: &LaserSheet, y0: f64, y1: f64| -> Vec<i64> {
        s.engrave
            .iter()
            .filter(|l| l.len() == 4 && l.first() == l.last() && l.iter().all(|p| p.y >= y0 && p.y <= y1))
            .map(|l| (l[0].x * 10.0).round() as i64)
            .collect()
    };
    assert_ne!(mark_x(&film, 11.0, 22.5), mark_x(&card, 11.0, 22.5), "bridge marks move with the material");
    assert_ne!(mark_x(&film, 49.0, 60.0), mark_x(&card, 49.0, 60.0), "corner marks too");
}
