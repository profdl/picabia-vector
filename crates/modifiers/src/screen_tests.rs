//! Tests for the Screen halftone core.

use vectorcraft_geom::{Point, Rect};

use crate::contour::{MAX_CELLS, Ring};
use crate::screen::{DEFAULT_LPI, MAX_LPI, MIN_LPI, Screen, SpotShape, ToneImage};

fn area(rings: &[Ring]) -> f64 {
    rings.iter().map(Ring::signed_area).sum()
}

/// `n × n` whole cells of `s` (angle 0: cells centred on multiples of the period).
fn cells(s: &Screen, n: f64) -> Rect {
    let p = s.period();
    Rect::new(-p * 0.5, -p * 0.5, p * (n - 0.5), p * (n - 0.5))
}

#[test]
fn ink_share_follows_the_tone_for_every_shape() {
    for shape in SpotShape::ALL {
        let s = Screen::new(shape, 18.0, 0.0, 24);
        let b = cells(&s, 6.0);
        for tone in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let t = s.trace(b, |_| tone).unwrap();
            assert!(!t.coarsened);
            let share = area(&t.rings) / b.area();
            assert!((share - tone).abs() < 0.02, "{shape:?} at {tone}: {share}");
        }
    }
}

#[test]
fn no_ink_at_zero_and_solid_at_one() {
    for shape in SpotShape::ALL {
        let s = Screen::new(shape, 10.0, 15.0, 8);
        let b = Rect::new(0.0, 0.0, 50.0, 40.0);
        assert!(s.trace(b, |_| 0.0).unwrap().rings.is_empty(), "{shape:?}");
        let full = s.trace(b, |_| 1.0).unwrap();
        assert_eq!(full.rings.len(), 1, "{shape:?}");
        // The grid covers the bounds, reaching under a step past the right and bottom.
        let a = area(&full.rings);
        assert!(a >= b.area() - 1e-6 && a <= (b.width() + full.step) * (b.height() + full.step), "{a}");
        // Junk tones count as no ink.
        assert!(s.trace(b, |_| f64::NAN).unwrap().rings.is_empty());
    }
}

#[test]
fn round_dots_merge_at_mid_grey_and_turn_into_holes() {
    let s = Screen::new(SpotShape::Round, 18.0, 0.0, 24);
    let b = cells(&s, 5.0);
    let light = s.trace(b, |_| 0.3).unwrap().rings;
    assert_eq!(light.len(), 25, "one dot per cell");
    assert!(light.iter().all(|r| !r.is_hole()));
    let dark = s.trace(b, |_| 0.8).unwrap().rings;
    let holes = dark.iter().filter(|r| r.is_hole()).count();
    // One hole at each inner cell corner (4 × 4), the art one inked piece round them.
    assert_eq!(holes, 16, "{} rings", dark.len());
    assert_eq!(dark.len() - holes, 1);
}

#[test]
fn threshold_rises_with_tone() {
    for shape in SpotShape::ALL {
        let s = Screen::new(shape, 20.0, 0.0, 12);
        let mut last = f64::NEG_INFINITY;
        for k in 0..=100 {
            let t = s.threshold(k as f64 / 100.0);
            assert!(t >= last, "{shape:?} at {k}");
            last = t;
        }
        assert!(s.threshold(-5.0) < s.threshold(0.01));
        assert!(s.threshold(f64::NAN) == s.threshold(0.0));
    }
}

#[test]
fn spot_is_periodic_and_turns_with_the_angle() {
    let s = Screen::new(SpotShape::Ellipse, 12.0, 30.0, 12);
    let p = s.period();
    let r = 30f64.to_radians();
    let (ax, ay) = (Point::new(r.cos(), r.sin()), Point::new(-r.sin(), r.cos()));
    let q = Point::new(3.3, -1.7);
    for (i, j) in [(1.0, 0.0), (0.0, 1.0), (-2.0, 3.0)] {
        let shifted = q + (ax.to_vec2() * i + ay.to_vec2() * j) * p;
        assert!((s.spot_at(q) - s.spot_at(shifted)).abs() < 1e-9);
    }
    // Lines run along the angle: moving along it keeps the value.
    let l = Screen::new(SpotShape::Line, 12.0, 30.0, 12);
    assert!((l.spot_at(q) - l.spot_at(q + ax.to_vec2() * 0.37 * p)).abs() < 1e-9);
}

#[test]
fn params_are_clamped() {
    assert_eq!(Screen::new(SpotShape::Round, f64::NAN, f64::INFINITY, 0).lpi(), DEFAULT_LPI);
    assert_eq!(Screen::new(SpotShape::Round, 0.0, 0.0, 0).lpi(), MIN_LPI);
    assert_eq!(Screen::new(SpotShape::Round, 1e9, 1e9, 1000).lpi(), MAX_LPI);
    assert_eq!(Screen::new(SpotShape::Round, 1e9, 1e9, 1000).angle_deg(), 360.0);
    assert_eq!(Screen::new(SpotShape::Round, 10.0, f64::NAN, 1000).angle_deg(), 0.0);
}

#[test]
fn huge_art_is_coarsened_to_the_cap() {
    let s = Screen::new(SpotShape::Round, MAX_LPI, 0.0, 64);
    let b = Rect::new(0.0, 0.0, 5000.0, 3000.0);
    let (g, coarsened) = s.grid(b, |_| 0.5).unwrap();
    assert!(coarsened);
    assert!((g.cols() - 1) * (g.rows() - 1) <= MAX_CELLS);
    // Bounds that aren't usable are refused, not traced.
    for bad in [Rect::new(0.0, 0.0, 0.0, 10.0), Rect::new(f64::NAN, 0.0, 1.0, 1.0), Rect::new(0.0, 0.0, f64::INFINITY, 1.0)] {
        assert!(s.trace(bad, |_| 0.5).is_err());
    }
    assert!(s.step_for(Rect::new(0.0, 0.0, 1e300, 1e300)).is_some_and(|(st, c)| st.is_finite() && c));
}

#[test]
fn tone_image_samples_darkness() {
    let place = Rect::new(0.0, 0.0, 20.0, 10.0);
    // 2 × 1: black then white.
    let img = ToneImage::from_luma8(2, 1, &[0, 255], place).unwrap();
    assert!((img.darkness_at(Point::new(2.0, 5.0)) - 1.0).abs() < 1e-6);
    assert!(img.darkness_at(Point::new(18.0, 5.0)).abs() < 1e-6);
    assert!((img.darkness_at(Point::new(10.0, 5.0)) - 0.5).abs() < 1e-6);
    assert_eq!(img.darkness_at(Point::new(-1.0, 5.0)), 0.0);
    assert_eq!(img.darkness_at(Point::new(f64::NAN, 5.0)), 0.0);
    // RGBA: transparent black is no ink; opaque red is its luminance's darkness.
    let rgba = ToneImage::from_rgba8(2, 1, &[0, 0, 0, 0, 255, 0, 0, 255], place).unwrap();
    assert_eq!(rgba.darkness_at(Point::new(1.0, 5.0)), 0.0);
    assert!((rgba.darkness_at(Point::new(19.0, 5.0)) - (1.0 - 0.2126)).abs() < 1e-3);
}

#[test]
fn tone_image_refuses_junk() {
    let place = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(ToneImage::from_luma8(0, 0, &[], place).is_none());
    assert!(ToneImage::from_luma8(2, 2, &[0; 3], place).is_none());
    assert!(ToneImage::from_luma8(usize::MAX, 2, &[], place).is_none());
    assert!(ToneImage::from_luma8(5000, 5000, &[], place).is_none());
    assert!(ToneImage::from_luma8(1, 1, &[0], Rect::new(0.0, 0.0, 0.0, 1.0)).is_none());
    assert!(ToneImage::from_luma8(1, 1, &[0], Rect::new(0.0, 0.0, f64::NAN, 1.0)).is_none());
    assert!(ToneImage::from_rgba8(1, 1, &[0; 3], place).is_none());
    assert!(ToneImage::from_rgba8(usize::MAX, usize::MAX, &[], place).is_none());
}

#[test]
fn an_image_drives_the_screen() {
    // A left-to-right ramp from white to black: ink grows to the right.
    let w = 64;
    let luma: Vec<u8> = (0..w).map(|i| (255 - i * 255 / (w - 1)) as u8).collect();
    let place = Rect::new(0.0, 0.0, 100.0, 20.0);
    let img = ToneImage::from_luma8(w, 1, &luma, place).unwrap();
    let s = Screen::new(SpotShape::Round, 36.0, 45.0, 12);
    let t = s.trace(place, |p| img.darkness_at(p)).unwrap();
    let share = area(&t.rings) / place.area();
    assert!((share - 0.5).abs() < 0.05, "{share}");
    let (left, right): (Vec<&Ring>, Vec<&Ring>) = t.rings.iter().partition(|r| r.points[0].x < 50.0);
    let a = |v: &[&Ring]| v.iter().map(|r| r.signed_area()).sum::<f64>();
    assert!(a(&right) > 2.0 * a(&left));
}

#[test]
fn tracing_is_deterministic() {
    let s = Screen::new(SpotShape::Cross, 24.0, 22.5, 10);
    let b = Rect::new(-10.0, 5.0, 60.0, 50.0);
    let tone = |p: Point| (p.x / 70.0).clamp(0.0, 1.0);
    assert_eq!(s.trace(b, tone).unwrap(), s.trace(b, tone).unwrap());
}
