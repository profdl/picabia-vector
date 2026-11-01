//! Displace: move every anchor by seeded fractal simplex noise (spatial) or by a radial sine
//! ripple round the centre of the anchors (radial).
//!
//! Ported from tlprocedural's Noise modifier (`NoiseModifierProcessor.displacePolygon`, its
//! simplex after Stefan Gustavson's public-domain description, its fractal sum and its radial
//! ripple). Differences, on purpose: each anchor moves with its handles, so curves stay curves
//! (tlprocedural flattened shapes to polygons first: a circle became 32 straight sides; put a
//! Resample row above for more points); the permutation table's LCG is exact integer arithmetic
//! (tlprocedural's multiplied past 2^53 in doubles and lost bits, so a seed gives a different,
//! equally stable, pattern here); the radial phase comes from the crate's seeded stream per copy
//! (tlprocedural's LCG of `seed + index` alternated between two phases); and every step is plain arithmetic (no `sin`, `cos` or `atan2`
//! from the platform), so the result is the same on every platform and in wasm.

use vectorcraft_geom::{Anchor, Point, SubPath, Vec2};

use crate::model::{Displace, DisplaceMode};

/// tlprocedural's LCG multiplier and increment (`buildPermTable`).
const LCG_A: u64 = 1_103_515_245;
const LCG_C: u64 = 12_345;
const LCG_MASK: u64 = 0x7fff_ffff;

/// The most octaves and ripples a field runs (the params' ranges).
const MAX_OCTAVES: u32 = 6;
const MAX_RIPPLES: u32 = 12;

/// The eight gradient directions of tlprocedural's 2D simplex.
const GRAD2: [(f64, f64); 8] = [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];

/// A displacement ready to run on one object (or one copy): its params clamped, its amount after
/// the progressive ramp, and what it reads from the whole object (the radial centre).
pub(super) struct Field {
    amplitude: f64,
    how: How,
}

enum How {
    Spatial { scale: f64, octaves: u32, perm: Vec<u8> },
    Radial { centre: Point, ripples: u32, phase_cos: f64, phase_sin: f64 },
}

impl Field {
    /// `p` (clamped) for an object whose anchors centre on `centre` (read in radial mode only); `copy`: the copy's index and
    /// the number of copies when the row runs on the copies (`None`: on the object).
    pub fn new(p: &Displace, centre: Point, copy: Option<(usize, usize)>) -> Field {
        let amplitude = p.amplitude * ramp(p, copy);
        // The loops are capped here too: `GeometryOp` and `shape_path` are public, so `p` may not
        // have been through `clamped`.
        let how = match p.mode {
            DisplaceMode::Spatial => How::Spatial { scale: p.scale, octaves: p.octaves.min(MAX_OCTAVES), perm: perm_table(p.seed) },
            DisplaceMode::Radial => {
                let (phase_sin, phase_cos) = sin_cos(phase(p.seed, copy.map_or(0, |c| c.0)));
                How::Radial { centre, ripples: p.ripple_count.min(MAX_RIPPLES), phase_cos, phase_sin }
            }
        };
        Field { amplitude, how }
    }

    /// `sp` with every anchor moved (its handles with it); `None` when nothing moves.
    pub fn displace(&self, sp: &SubPath) -> Option<SubPath> {
        if self.amplitude == 0.0 || !self.amplitude.is_finite() || sp.anchors.is_empty() {
            return None;
        }
        let anchors = sp
            .anchors
            .iter()
            .map(|a| {
                let d = self.offset(a.p);
                Anchor { p: a.p + d, h_in: a.h_in + d, h_out: a.h_out + d, kind: a.kind }
            })
            .collect();
        Some(SubPath::new(anchors, sp.closed))
    }

    /// How far the anchor at `p` moves.
    fn offset(&self, p: Point) -> Vec2 {
        match &self.how {
            How::Spatial { scale, octaves, perm } => {
                let (x, y) = (p.x * scale, p.y * scale);
                let nx = fbm(x, y, *octaves, perm);
                let ny = fbm(x + 100.0, y + 100.0, *octaves, perm);
                Vec2::new(nx * self.amplitude, ny * self.amplitude)
            }
            How::Radial { centre, ripples, phase_cos, phase_sin } => {
                let v = p - *centre;
                // Plain √ (correctly rounded everywhere), not the platform's `hypot`.
                let dist = (v.x * v.x + v.y * v.y).sqrt();
                if dist <= 0.0 || !dist.is_finite() {
                    return Vec2::ZERO;
                }
                // sin(ripples·θ + phase), θ the anchor's angle round the centre: the unit vector
                // raised to the ripples-th power (complex), turned by the phase.
                let (c, s) = (v.x / dist, v.y / dist);
                let (mut re, mut im) = (1.0, 0.0);
                for _ in 0..*ripples {
                    (re, im) = (re * c - im * s, re * s + im * c);
                }
                let wave = im * phase_cos + re * phase_sin;
                // tlprocedural: bump = dist · amplitude/100 · 0.25 · wave, along the anchor's
                // direction from the centre.
                v * (self.amplitude / 100.0 * 0.25 * wave)
            }
        }
    }
}

/// The share of the amplitude for copy `copy` (index, count): tlprocedural's
/// `computeProgressiveScale`: none on the first copy, `progressiveAmount` on the last, in a
/// straight line (reversed with `progressiveReverse`); all of it on the object, on a single copy
/// or when not progressive.
pub(super) fn ramp(p: &Displace, copy: Option<(usize, usize)>) -> f64 {
    let Some((index, count)) = copy else { return 1.0 };
    if !p.progressive || count < 2 {
        return 1.0;
    }
    let t = (index.min(count - 1) as f64) / ((count - 1) as f64);
    let directed = if p.progressive_reverse { 1.0 - t } else { t };
    directed * p.progressive_amount
}

/// tlprocedural's seeded permutation table: 0..=255 shuffled by Fisher-Yates driven by its LCG,
/// repeated once (512 entries).
pub(super) fn perm_table(seed: u32) -> Vec<u8> {
    let mut source: Vec<u8> = (0..=255u8).collect();
    let mut s = u64::from(seed);
    for i in (1..256usize).rev() {
        s = s.wrapping_mul(LCG_A).wrapping_add(LCG_C) & LCG_MASK;
        let j = usize::try_from(s % (i as u64 + 1)).unwrap_or(0);
        source.swap(i, j.min(i));
    }
    let mut perm = source.clone();
    perm.extend_from_slice(&source);
    perm
}

/// The radial ripple's phase for `seed` and copy `index`, in radians: the crate's seeded stream
/// for `(seed, index)`, scaled to a turn. (tlprocedural took one LCG step of `seed + index`: a
/// step's increment is a fixed 0.514 of a turn, so consecutive copies alternated between two
/// nearly equal phases instead of each getting its own.)
pub(super) fn phase(seed: u32, index: usize) -> f64 {
    let unit = (crate::kinds::rng_stream(seed, index).signed() + 1.0) / 2.0;
    unit * std::f64::consts::TAU
}

fn perm(perm: &[u8], i: usize) -> usize {
    usize::from(perm.get(i).copied().unwrap_or(0))
}

/// 2D simplex noise in about -1..1 (tlprocedural's `simplex2D`); 0 where `x` or `y` isn't finite.
pub(super) fn simplex(x: f64, y: f64, p: &[u8]) -> f64 {
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let sqrt3 = 3f64.sqrt();
    let f2 = 0.5 * (sqrt3 - 1.0);
    let g2 = (3.0 - sqrt3) / 6.0;
    let s = (x + y) * f2;
    let (i, j) = ((x + s).floor(), (y + s).floor());
    let t = (i + j) * g2;
    let (x0, y0) = (x - (i - t), y - (j - t));
    let (i1, j1): (usize, usize) = if x0 > y0 { (1, 0) } else { (0, 1) };
    let (x1, y1) = (x0 - i1 as f64 + g2, y0 - j1 as f64 + g2);
    let (x2, y2) = (x0 - 1.0 + 2.0 * g2, y0 - 1.0 + 2.0 * g2);
    // The cell's corner mod 256 (`i & 255` in tlprocedural); a saturated cast is still 0..=255.
    let ii = ((i as i64) & 255) as usize;
    let jj = ((j as i64) & 255) as usize;
    let corner = |gi: usize, x: f64, y: f64| -> f64 {
        let t = 0.5 - x * x - y * y;
        if t <= 0.0 {
            return 0.0;
        }
        let t = t * t;
        let (gx, gy) = GRAD2.get(gi % 8).copied().unwrap_or((0.0, 0.0));
        t * t * (gx * x + gy * y)
    };
    let n0 = corner(perm(p, ii + perm(p, jj)), x0, y0);
    let n1 = corner(perm(p, ii + i1 + perm(p, jj + j1)), x1, y1);
    let n2 = corner(perm(p, ii + 1 + perm(p, jj + 1)), x2, y2);
    70.0 * (n0 + n1 + n2)
}

/// `octaves` layers of simplex noise, each twice the frequency and half the weight, normalised
/// by the weights (tlprocedural's `fbmNoise2D`).
pub(super) fn fbm(x: f64, y: f64, octaves: u32, p: &[u8]) -> f64 {
    let (mut value, mut weight, mut frequency, mut total) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves.clamp(1, MAX_OCTAVES) {
        value += simplex(x * frequency, y * frequency, p) * weight;
        total += weight;
        weight *= 0.5;
        frequency *= 2.0;
    }
    value / total
}

/// `(sin x, cos x)` by Taylor series after reducing `x` to -π..π: plain arithmetic, so the same
/// bits on every platform (the platform's `sin` may differ in the last place). `x` is a phase in
/// 0..2π here.
pub(super) fn sin_cos(x: f64) -> (f64, f64) {
    use std::f64::consts::{PI, TAU};
    if !x.is_finite() {
        return (0.0, 1.0);
    }
    let mut r = x % TAU;
    if r > PI {
        r -= TAU;
    } else if r < -PI {
        r += TAU;
    }
    let (mut sin, mut cos) = (0.0, 0.0);
    let (mut s_term, mut c_term) = (r, 1.0);
    let r2 = r * r;
    for n in 0..30u32 {
        sin += s_term;
        cos += c_term;
        let k = f64::from(2 * n + 1);
        s_term *= -r2 / ((k + 1.0) * (k + 2.0));
        c_term *= -r2 / (k * (k + 1.0));
    }
    (sin, cos)
}
