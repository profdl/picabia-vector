//! The single-stroke laser font.

use vectorcraft_geom::kurbo::Point;

use crate::laser_font::glyph_table;
use crate::{font_has, text_polylines};

#[test]
fn every_glyph_draws_within_its_advance() {
    let cap = 6.0; // 1 mm per grid unit
    let mut n = 0;
    for (c, w) in glyph_table() {
        let (lines, width) = text_polylines(&c.to_string(), Point::new(0.0, 6.0), cap);
        assert!(!lines.is_empty() && lines.iter().all(|l| l.len() >= 2), "{c:?} has strokes");
        assert!((width - w).abs() < 1e-9, "{c:?}: {width} vs {w}");
        for p in lines.iter().flatten() {
            assert!(p.x >= -1e-9 && p.x <= w + 1e-9, "{c:?} x {} within 0..{w}", p.x);
            assert!(p.y >= -1e-9 && p.y <= 7.0 + 1e-9, "{c:?} y {}", p.y);
        }
        let ink: f64 = lines.iter().flat_map(|l| l.windows(2)).map(|s| (s[1] - s[0]).hypot()).sum();
        assert!(ink > 0.4, "{c:?} visible");
        n += 1;
    }
    assert!(n >= 52, "{n} glyphs");
}

#[test]
fn the_contract_characters_are_covered() {
    for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .,-_/:#()×x+=%'\"↑°".chars() {
        assert!(font_has(c), "{c:?}");
    }
    for c in ['é', '✓', '€', '\n'] {
        assert!(!font_has(c), "{c:?}");
    }
    // Lowercase draws uppercase; unknown characters draw a box.
    assert_eq!(text_polylines("abc", Point::ZERO, 2.5), text_polylines("ABC", Point::ZERO, 2.5));
    let (boxed, w) = text_polylines("✓", Point::ZERO, 6.0);
    assert_eq!(boxed.len(), 1);
    assert!(w > 0.0);
    // A distinct shape per character (no two glyphs alike).
    let shapes: Vec<_> = glyph_table().map(|(c, _)| format!("{:?}", text_polylines(&c.to_string(), Point::ZERO, 6.0).0)).collect();
    for (i, a) in shapes.iter().enumerate() {
        assert!(shapes.iter().skip(i + 1).all(|b| b != a), "glyph {i} is duplicated");
    }
}

#[test]
fn text_is_laid_out_on_the_baseline() {
    let (lines, w) = text_polylines("A B", Point::new(10.0, 20.0), 2.5);
    let s = 2.5 / 6.0;
    assert!((w - (4.0 + 1.5 + 2.0 + 1.5 + 4.0) * s).abs() < 1e-9, "{w}");
    let ys: Vec<f64> = lines.iter().flatten().map(|p| p.y).collect();
    assert!(ys.iter().all(|&y| (17.5 - 1e-9..=20.0 + 1e-9).contains(&y)), "cap top 17.5, baseline 20");
    assert!(lines.iter().flatten().all(|p| p.x >= 10.0 - 1e-9 && p.x <= 10.0 + w + 1e-9));
    assert_eq!(text_polylines("", Point::ZERO, 2.5), (Vec::new(), 0.0));
    assert!(text_polylines("A", Point::ZERO, f64::NAN).0.is_empty());
    assert!(text_polylines("A", Point::new(f64::INFINITY, 0.0), 2.0).0.is_empty());
    assert!(text_polylines("A", Point::ZERO, -1.0).0.is_empty());
    let long = "W".repeat(10_000);
    let (_, w) = text_polylines(&long, Point::ZERO, 6.0);
    assert!((w - (crate::MAX_TEXT_CHARS as f64 * 6.5 - 1.5)).abs() < 1e-6, "capped: {w}");
}
