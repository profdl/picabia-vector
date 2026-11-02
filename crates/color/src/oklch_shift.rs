//! PICABIA: the Color modifier row's colour shift (`docs/picabia/colour.md` › The maths).
//!
//! A shift moves a colour's OKLCH hue, chroma and lightness by percentages: hue 100 = a full turn,
//! chroma 100 = +0.4, lightness 100 = +1. A hue shift on a near-neutral colour (chroma under 0.02)
//! gives it some chroma so the shift shows. A result outside sRGB keeps its lightness and hue and
//! loses chroma (a fixed 20-step bisection) until its linear channels fit in 0..=1.
//!
//! Plain `f64` arithmetic in a fixed order: the same input gives the same bits on every platform
//! that rounds the standard functions alike.

use crate::Color;
use crate::libraries::{oklab_to_linear_srgb, srgb_to_oklab};

/// The most OKLCH chroma a shift produces.
pub const MAX_CHROMA: f64 = 0.4;
/// Below this chroma a colour counts as neutral: a hue shift injects chroma.
const NEUTRAL_CHROMA: f64 = 0.02;
/// Bisection steps when reducing chroma into sRGB.
const GAMUT_STEPS: usize = 20;
/// How far outside 0..=1 a linear channel may be and still count as in gamut.
const GAMUT_TOLERANCE: f64 = 1e-6;

/// A colour shift in percent (each −100..=100 in the row; any finite value works here).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OklchShift {
    /// 100 = a full turn (360°).
    pub hue: f64,
    /// 100 = +0.4 OKLCH chroma.
    pub chroma: f64,
    /// 100 = +1 OKLCH lightness.
    pub lightness: f64,
}

impl OklchShift {
    pub const ZERO: OklchShift = OklchShift { hue: 0.0, chroma: 0.0, lightness: 0.0 };

    pub fn new(hue: f64, chroma: f64, lightness: f64) -> OklchShift {
        OklchShift { hue, chroma, lightness }
    }

    /// Whether it changes nothing (every channel 0, or not finite).
    pub fn is_zero(&self) -> bool {
        [self.hue, self.chroma, self.lightness].iter().all(|v| *v == 0.0 || !v.is_finite())
    }

    /// The channels as bits, for cache keys.
    pub fn key(&self) -> [u64; 3] {
        [self.hue.to_bits(), self.chroma.to_bits(), self.lightness.to_bits()]
    }
}

/// Below this chroma a colour has no hue (its `a` and `b` are rounding noise): hue 0.
const ACHROMATIC: f64 = 1e-6;

/// OKLCH `[l, c, h°]` (h in 0..360; 0 for an achromatic colour) of sRGB `rgb` (0..=1, gamma
/// encoded).
pub fn srgb_to_oklch(rgb: [f32; 3]) -> [f64; 3] {
    let [l, a, b] = srgb_to_oklab(rgb);
    let c = a.hypot(b);
    let h = b.atan2(a).to_degrees().rem_euclid(360.0);
    [l, c, if h.is_finite() && c >= ACHROMATIC { h } else { 0.0 }]
}

/// Linear sRGB of OKLCH `(l, c, h°)`.
fn linear_of(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (sin, cos) = h.to_radians().sin_cos();
    oklab_to_linear_srgb(l, c * cos, c * sin)
}

fn fits(lin: [f64; 3]) -> bool {
    lin.iter().all(|v| (-GAMUT_TOLERANCE..=1.0 + GAMUT_TOLERANCE).contains(v))
}

fn encode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB (0..=1, gamma encoded) of OKLCH `(l, c, h°)`, chroma reduced into sRGB.
pub fn oklch_to_srgb_in_gamut(l: f64, c: f64, h: f64) -> [f64; 3] {
    let lin = if fits(linear_of(l, c, h)) {
        linear_of(l, c, h)
    } else {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..GAMUT_STEPS {
            let mid = (lo + hi) / 2.0;
            if fits(linear_of(l, mid, h)) { lo = mid } else { hi = mid }
        }
        linear_of(l, lo, h)
    };
    lin.map(encode)
}

/// sRGB `rgb` (0..=1, gamma encoded) shifted by `s`; unchanged (the same bits) when `s` is zero.
pub fn shift_srgb(rgb: [f32; 3], s: OklchShift) -> [f32; 3] {
    if s.is_zero() || !rgb.iter().all(|v| v.is_finite()) {
        return rgb;
    }
    let fin = |v: f64| if v.is_finite() { v } else { 0.0 };
    let (sh, sc, sl) = (fin(s.hue), fin(s.chroma), fin(s.lightness));
    let [l, c, h] = srgb_to_oklch(rgb.map(|v| v.clamp(0.0, 1.0)));
    let l = (l + sl / 100.0).clamp(0.0, 1.0);
    let mut c = (c + sc / 100.0 * MAX_CHROMA).clamp(0.0, MAX_CHROMA);
    let h = (h + sh / 100.0 * 360.0).rem_euclid(360.0);
    if sh != 0.0 && c < NEUTRAL_CHROMA {
        c = MAX_CHROMA.min((sh / 100.0).abs() * 0.2);
    }
    oklch_to_srgb_in_gamut(l, c, h).map(|v| v as f32)
}

/// `c` shifted by `s`, as an RGB colour (its display colour shifted; [`Color::to_rgb`]). The
/// caller puts it back in the colour model it wants.
pub fn shift_color(c: Color, s: OklchShift) -> Color {
    if s.is_zero() {
        return c;
    }
    let [r, g, b] = shift_srgb(c.to_rgb(), s);
    Color::rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(rgb: [f32; 3]) -> String {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", q(rgb[0]), q(rgb[1]), q(rgb[2]))
    }

    /// A tiny reference for the formulas, written out step by step from the spec.
    fn reference(rgb: [f32; 3], sh: f64, sc: f64, sl: f64) -> [f64; 3] {
        let [l0, a0, b0] = srgb_to_oklab(rgb);
        let c0 = (a0 * a0 + b0 * b0).sqrt();
        let mut h0 = if c0 < 1e-6 { 0.0 } else { b0.atan2(a0).to_degrees() };
        if h0 < 0.0 {
            h0 += 360.0;
        }
        let l = (l0 + sl / 100.0).clamp(0.0, 1.0);
        let mut c = (c0 + sc / 100.0 * 0.4).clamp(0.0, 0.4);
        let mut h = (h0 + sh / 100.0 * 360.0) % 360.0;
        if h < 0.0 {
            h += 360.0;
        }
        if sh != 0.0 && c < 0.02 {
            c = (sh.abs() / 100.0 * 0.2).min(0.4);
        }
        [l, c, h]
    }

    fn oklch_of(rgb: [f32; 3]) -> [f64; 3] {
        srgb_to_oklch(rgb)
    }

    #[test]
    fn zero_shift_is_bit_for_bit() {
        let rgb = [0.123_456_78, 0.5, 0.987_654_3];
        assert_eq!(shift_srgb(rgb, OklchShift::ZERO), rgb);
        let c = Color::cmyk(0.1, 0.2, 0.3, 0.4);
        assert_eq!(shift_color(c, OklchShift::ZERO), c);
    }

    #[test]
    fn red_half_turn_keeps_lightness_and_turns_hue() {
        let red = [1.0, 0.0, 0.0];
        let [l, _, h] = reference(red, 50.0, 0.0, 0.0);
        let out = shift_srgb(red, OklchShift::new(50.0, 0.0, 0.0));
        let [ol, oc, oh] = oklch_of(out);
        assert!((ol - l).abs() < 2e-3, "lightness {ol} vs {l}");
        assert!((oh - h).abs() < 0.5, "hue {oh} vs {h}");
        assert!(oc > 0.05 && oc < 0.26, "chroma {oc} reduced into sRGB");
        // Golden: red's hue (29.2°) plus 180°, chroma reduced to fit.
        assert_eq!(hex(out), "#009aac", "{out:?}");
    }

    #[test]
    fn white_hue_shift_injects_chroma() {
        let white = [1.0, 1.0, 1.0];
        let [_, c, _] = reference(white, 10.0, 0.0, 0.0);
        assert!((c - 0.02).abs() < 1e-12);
        let out = shift_srgb(white, OklchShift::new(10.0, 0.0, 0.0));
        // At L = 1 no chroma fits: the bisection brings it back to (nearly) white.
        assert_eq!(hex(out), "#ffffff");
        // A mid grey shows the injected chroma.
        let grey = shift_srgb([0.5, 0.5, 0.5], OklchShift::new(10.0, 0.0, 0.0));
        let [_, gc, _] = oklch_of(grey);
        assert!((gc - 0.02).abs() < 2e-3, "chroma {gc}");
        assert_eq!(hex(grey), "#8b7c78");
    }

    #[test]
    fn black_darker_clamps_at_zero() {
        let out = shift_srgb([0.0, 0.0, 0.0], OklchShift::new(0.0, 0.0, -10.0));
        assert_eq!(out, [0.0, 0.0, 0.0]);
        let dark = shift_srgb([0.05, 0.05, 0.05], OklchShift::new(0.0, 0.0, -10.0));
        assert_eq!(hex(dark), "#010101");
    }

    #[test]
    fn saturated_blue_chroma_clamps_and_keeps_hue() {
        let blue = [0.1, 0.2, 0.9];
        let [_, c, h] = reference(blue, 0.0, 100.0, 0.0);
        assert_eq!(c, 0.4);
        let out = shift_srgb(blue, OklchShift::new(0.0, 100.0, 0.0));
        let [_, _, oh] = oklch_of(out);
        let dh = (oh - h).abs().min(360.0 - (oh - h).abs());
        assert!(dh < 0.5, "hue {oh} vs {h}");
        assert!(out.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn lightness_raises_l() {
        let c = [0.2, 0.4, 0.6];
        let [l, ..] = reference(c, 0.0, 0.0, 10.0);
        let out = shift_srgb(c, OklchShift::new(0.0, 0.0, 10.0));
        assert!((oklch_of(out)[0] - l).abs() < 2e-3);
    }

    #[test]
    fn junk_shifts_never_panic() {
        for s in [f64::NAN, f64::INFINITY, -1e300, 1e300] {
            let out = shift_srgb([0.3, 0.6, 0.1], OklchShift::new(s, s, s));
            assert!(out.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{out:?}");
        }
        let out = shift_srgb([f32::NAN, 0.0, 2.0], OklchShift::new(10.0, 0.0, 0.0));
        assert!(out[0].is_nan());
    }
}
