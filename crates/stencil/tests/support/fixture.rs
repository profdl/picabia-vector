//! Synthetic stencil fixtures (shared by the unit tests and the integration tests): a traced-style
//! design of stacked flat colours, built from a seed, with every case the clean-up is for.
//!
//! - a dark ground over the whole design;
//! - jagged "traced" blobs (radial noise on every point, hundreds of points each) in three flat
//!   colours, stacked so they overlap;
//! - nested shapes: a light ring with a dark disc inside it and a light dot inside that;
//! - specks: 0.35-unit squares turned 45° (the size a tracer leaves behind);
//! - small text-like glyphs: letter shapes made of bars, half of them too thin to cut.
#![allow(dead_code)]

use vectorcraft_geom::kurbo::{Affine, BezPath, Circle, Point, Rect, Shape};
use vectorcraft_stencil::Paint;

/// The design's bounds (a 920 × 752 artboard, as a traced import).
pub const BOUNDS: Rect = Rect::new(0.0, 0.0, 920.0, 752.0);
pub const DARK: [f32; 4] = [0x28 as f32 / 255.0, 0x26 as f32 / 255.0, 0x2e as f32 / 255.0, 1.0];
pub const MID: [f32; 4] = [0xcb as f32 / 255.0, 0xab as f32 / 255.0, 0x9e as f32 / 255.0, 1.0];
pub const LIGHT: [f32; 4] = [0xe7 as f32 / 255.0, 0xcb as f32 / 255.0, 0x5b as f32 / 255.0, 1.0];
/// Glyphs sit below this line on the bare ground; blobs and nests above it.
pub const GLYPH_TOP: f64 = 660.0;
pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// What [`design`] put in, to check against.
#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub blobs: usize,
    pub specks: usize,
    /// Specks in a colour that the default threshold cuts (light, white).
    pub light_specks: usize,
    /// Glyphs 9 units tall with 1.2-unit bars (kept at the default size and limits).
    pub glyphs: Vec<Rect>,
    /// Glyphs 3 units tall with 0.5-unit bars (narrower than the default minimum cut width).
    pub tiny_glyphs: Vec<Rect>,
    pub nests: usize,
}

/// A small deterministic PRNG (SplitMix64).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [a, b).
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
}

/// A jagged closed blob around `c`: `n` points at radius `r` with a smooth wobble and per-point
/// jitter of `jag` (the staircase a tracer leaves on an edge).
pub fn jagged_blob(rng: &mut Rng, c: Point, r: f64, n: usize, jag: f64) -> BezPath {
    let (k1, k2) = (rng.range(2.0, 5.0).floor(), rng.range(0.0, std::f64::consts::TAU));
    let mut p = BezPath::new();
    for i in 0..n {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        let rr = r * (1.0 + 0.25 * (k1 * t + k2).sin()) + rng.range(-jag, jag);
        let q = c + vectorcraft_geom::kurbo::Vec2::new(t.cos(), t.sin()) * rr.max(r * 0.2);
        if i == 0 { p.move_to(q) } else { p.line_to(q) }
    }
    p.close_path();
    p
}

/// A 0.35 × 0.35 square turned 45° at `c`.
pub fn speck(c: Point) -> BezPath {
    let sq = Rect::new(-0.175, -0.175, 0.175, 0.175).to_path(0.1);
    Affine::translate(c.to_vec2()) * Affine::rotate(std::f64::consts::FRAC_PI_4) * sq
}

/// A letter-like glyph ("E") `h` tall with bars `stroke` thick, top-left at `o`.
pub fn glyph(o: Point, h: f64, stroke: f64) -> BezPath {
    let w = h * 0.6;
    let mut p = BezPath::new();
    for r in [
        Rect::new(0.0, 0.0, stroke, h),
        Rect::new(0.0, 0.0, w, stroke),
        Rect::new(0.0, (h - stroke) / 2.0, w * 0.8, (h + stroke) / 2.0),
        Rect::new(0.0, h - stroke, w, h),
    ] {
        p.extend((Affine::translate(o.to_vec2()) * r.to_path(0.1)).iter());
    }
    p
}

/// A ring (outer `r`, inner `r · 0.6`) at `c`: one paint, two subpaths, the hole wound backwards.
pub fn ring(c: Point, r: f64) -> BezPath {
    let mut p = Circle::new(c, r).to_path(0.05);
    p.extend(BezPath::reverse_subpaths(&Circle::new(c, r * 0.6).to_path(0.05)).iter());
    p
}

/// A traced-style design of about `shapes` paints over [`BOUNDS`], from `seed`.
pub fn design(shapes: usize, seed: u64) -> (Vec<Paint>, Contents) {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    let mut c = Contents::default();
    let push = |out: &mut Vec<Paint>, path: BezPath, colour: [f32; 4]| {
        let order = out.len() as u32;
        out.push(Paint { path, colour, order });
    };
    push(&mut out, BOUNDS.to_path(0.1), DARK);
    let (w, h) = (BOUNDS.width(), BOUNDS.height());
    let blobs = shapes * 6 / 10;
    for i in 0..blobs {
        let colour = [DARK, MID, LIGHT][i % 3];
        // Inside the bounds (wobble and jitter included) and above the glyph strip.
        let r = rng.range(8.0, 70.0);
        let m = r * 1.3 + 2.0;
        let ctr = Point::new(rng.range(m, w - m), rng.range(m, GLYPH_TOP - m));
        let n = (r * 6.0) as usize + 40;
        push(&mut out, jagged_blob(&mut rng, ctr, r, n, 0.6), colour);
        c.blobs += 1;
    }
    let nests = (shapes / 30).max(1);
    for _ in 0..nests {
        let ctr = Point::new(rng.range(60.0, w - 60.0), rng.range(60.0, GLYPH_TOP - 60.0));
        push(&mut out, ring(ctr, 30.0), WHITE);
        push(&mut out, Circle::new(ctr, 12.0).to_path(0.05), DARK);
        push(&mut out, Circle::new(ctr, 5.0).to_path(0.05), WHITE);
        c.nests += 1;
    }
    let glyphs = shapes / 10;
    for i in 0..glyphs {
        let o = Point::new(20.0 + (i % 40) as f64 * 12.0, GLYPH_TOP + 10.0 + (i / 40) as f64 * 14.0);
        let (gh, stroke) = if i % 2 == 0 { (9.0, 1.2) } else { (3.0, 0.5) };
        let g = glyph(o, gh, stroke);
        let bbox = g.bounding_box();
        push(&mut out, g, WHITE);
        if i % 2 == 0 { c.glyphs.push(bbox) } else { c.tiny_glyphs.push(bbox) }
    }
    let specks = shapes.saturating_sub(out.len());
    for i in 0..specks {
        let colour = [DARK, MID, LIGHT, WHITE][i % 4];
        push(&mut out, speck(Point::new(rng.range(1.0, w - 1.0), rng.range(1.0, h - 1.0))), colour);
        c.specks += 1;
        if i % 4 >= 2 {
            c.light_specks += 1;
        }
    }
    (out, c)
}
