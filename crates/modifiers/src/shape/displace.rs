//! Displace: move every anchor by one of five patterns, along the pattern's own direction, the
//! path's normal, or an axis of the copy:
//! - smooth noise (`spatial`): seeded fractal simplex noise where the anchor sits;
//! - `radial`: a sine ripple round the centre of the anchors;
//! - `wave`: a sine, square, triangle or sawtooth wave across the object (or along its outline);
//! - `jitter`: a seeded random amount per anchor;
//! - `fractal`: subdivision growth, which adds anchors ([`super::fractal`]).
//!
//! Ported from tlprocedural (permission in NOTICE): the Noise modifier
//! (`NoiseModifierProcessor.displacePolygon`, its simplex after Stefan Gustavson's public-domain
//! description, its fractal sum and its radial ripple), the Wave modifier
//! (`WaveModifierProcessor.wavePolygon`, `computeWave`), Randomize Points
//! (`applyRandomizeToPolygon`) and Subdivision Growth (`SubdivisionGrowthProcessor`).
//! Differences, on purpose: each anchor moves with its handles, so curves stay curves
//! (tlprocedural flattened shapes to polygons first: a circle became 32 straight sides; put a
//! Resample row above for more points); the LCGs are exact integer arithmetic (tlprocedural's
//! multiplied past 2^53 in doubles and lost bits, so a seed gives a different, equally stable,
//! pattern here); the radial phase and the jitter's stream come from the crate's seeded stream per
//! copy (tlprocedural's LCG of `seed + index` alternated between two phases, and its jitter was
//! the same on every copy); `x`, `y` and `angle` are read in each copy's frame (as tlprocedural
//! waved each copy unrotated); and every step is plain arithmetic (no `sin`, `cos`, `atan2`,
//! `hypot` or `pow` from the platform), so the result is the same on every platform and in wasm.

use std::cell::Cell;

use vectorcraft_geom::{Anchor, Point, SubPath, Vec2};

use super::fractal;
use crate::model::{Displace, DisplaceDirection, DisplaceMode, Waveform};

/// tlprocedural's LCG multiplier and increment (`buildPermTable`, `createSeededRandom`).
const LCG_A: u64 = 1_103_515_245;
const LCG_C: u64 = 12_345;
const LCG_MASK: u64 = 0x7fff_ffff;

/// The most octaves and ripples a field runs (the params' ranges).
const MAX_OCTAVES: u32 = 6;
const MAX_RIPPLES: u32 = 12;

/// tlprocedural's `MIN_RANDOMIZE_LENGTH`: a jittered anchor this close to the centre stays put,
/// and none is pulled closer than this.
const MIN_JITTER_LENGTH: f64 = 1e-4;

/// The shortest extent a wave runs across (tlprocedural's 0.001).
const MIN_WAVE_SPAN: f64 = 0.001;

/// The eight gradient directions of tlprocedural's 2D simplex.
const GRAD2: [(f64, f64); 8] = [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];

/// A displacement ready to run on one object (or one copy): its params clamped, its amount after
/// the progressive ramp, and what it reads from the whole object (the centre, the wave's span).
pub(super) struct Field {
    amplitude: f64,
    how: How,
    aim: Aim,
    /// The anchors of the object displaced so far: a fractal seeds its segments by their place in
    /// the whole object, not just their subpath.
    seen: Cell<usize>,
}

/// Which way an anchor moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Aim {
    /// The pattern's own direction.
    Own,
    /// The path's outward normal.
    Normal,
    /// This unit vector, in document space.
    Along(Vec2),
}

enum How {
    Spatial {
        scale: f64,
        octaves: u32,
        perm: Vec<u8>,
    },
    Radial {
        centre: Point,
        ripples: u32,
        phase_cos: f64,
        phase_sin: f64,
        frame: Frame,
    },
    Wave {
        form: Waveform,
        cycles: f64,
        turns: f64,
        across: Across,
    },
    /// `state`: the LCG, carried from one anchor to the next across the object's subpaths.
    Jitter {
        centre: Point,
        least: f64,
        state: Cell<u64>,
    },
    Fractal(fractal::Grow),
}

/// What a wave runs across.
enum Across {
    /// Each subpath's outline, start to end (direction normal).
    Outline,
    /// The object's extent along `along` (document space), `lo` to `lo + len` once known.
    Span { along: Vec2, span: Option<(f64, f64)> },
}

/// The frame a copy's pattern is read in: its turn (the direction of its transform's x axis)
/// and whether it is reflected, so a rotated copy's ripple turns with it and a mirrored copy's is
/// the mirror image of its partner's (as Deform reads each copy in its frame). Found with plain
/// arithmetic, no `atan2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    cos: f64,
    sin: f64,
    flip: bool,
}

impl Frame {
    /// The object's own frame (no turn, no reflection).
    pub const IDENTITY: Frame = Frame { cos: 1.0, sin: 0.0, flip: false };

    /// The frame of copy transform `t`; the identity when its x axis has no direction.
    pub fn of(t: vectorcraft_geom::Affine) -> Frame {
        let [a, b, c, d, ..] = t.as_coeffs();
        let len = (a * a + b * b).sqrt();
        if !(len.is_finite() && len > 1e-12) {
            return Frame::IDENTITY;
        }
        let det = a * d - b * c;
        Frame { cos: a / len, sin: b / len, flip: det.is_finite() && det < 0.0 }
    }

    /// Whether the frame is reflected.
    pub fn flipped(&self) -> bool {
        self.flip
    }

    /// `v` read in the frame: turned back, then unreflected.
    fn local(&self, v: Vec2) -> (f64, f64) {
        let x = self.cos * v.x + self.sin * v.y;
        let y = self.cos * v.y - self.sin * v.x;
        (x, if self.flip { -y } else { y })
    }

    /// Local `(x, y)` in document space: reflected, then turned ([`Frame::local`]'s inverse).
    fn to_doc(self, x: f64, y: f64) -> Vec2 {
        let y = if self.flip { -y } else { y };
        Vec2::new(self.cos * x - self.sin * y, self.sin * x + self.cos * y)
    }
}

/// `(sin, cos)` of `deg` degrees ([`sin_cos`]); 0° when it isn't finite.
fn sin_cos_deg(deg: f64) -> (f64, f64) {
    sin_cos(if deg.is_finite() { deg % 360.0 } else { 0.0 } * std::f64::consts::PI / 180.0)
}

/// The axis `p.direction` names, in the copy's own axes (`None`: own or normal).
fn local_axis(p: &Displace) -> Option<(f64, f64)> {
    match p.direction {
        DisplaceDirection::Own | DisplaceDirection::Normal => None,
        DisplaceDirection::X => Some((1.0, 0.0)),
        DisplaceDirection::Y => Some((0.0, 1.0)),
        DisplaceDirection::Angle => {
            let (s, c) = sin_cos_deg(p.angle);
            Some((c, s))
        }
    }
}

/// The direction a wave runs across, in document space (it moves anchors across it); `None` when
/// it runs along the outline (direction normal) or `p` isn't a wave. tlprocedural's `axis`:
/// horizontal (its default, our own and `y`) runs across the width and moves in y, vertical (`x`)
/// down the height moving in x; an angle runs across it, 90° being `y`.
pub(super) fn wave_along(p: &Displace, frame: Frame) -> Option<Vec2> {
    if p.mode != DisplaceMode::Wave {
        return None;
    }
    let (x, y) = match p.direction {
        DisplaceDirection::Normal => return None,
        DisplaceDirection::Own | DisplaceDirection::Y => (1.0, 0.0),
        DisplaceDirection::X => (0.0, 1.0),
        DisplaceDirection::Angle => {
            let (s, c) = sin_cos_deg(p.angle);
            (s, -c)
        }
    };
    Some(frame.to_doc(x, y))
}

/// Whether `p` reads the centre of the object's anchors.
pub(super) fn needs_centre(p: &Displace) -> bool {
    match p.mode {
        DisplaceMode::Radial => true,
        DisplaceMode::Jitter => p.direction == DisplaceDirection::Own,
        _ => false,
    }
}

impl Field {
    /// `p` (clamped) for an object whose anchors centre on `centre` (read by radial and jitter);
    /// `copy`: the copy's place among the array copies and their number
    /// ([`crate::Output::ramp_position`]; `None`: the whole effect, phase and stream of place 0);
    /// `frame`: the copy's frame. A wave runs across nothing until [`Field::spanning`].
    pub fn new(p: &Displace, centre: Point, copy: Option<(usize, usize)>, frame: Frame) -> Field {
        let amplitude = p.amplitude * ramp(p, copy);
        let place = copy.map_or(0, |c| c.0);
        let mut aim = match (p.direction, local_axis(p)) {
            (DisplaceDirection::Normal, _) => Aim::Normal,
            (_, Some((x, y))) => Aim::Along(frame.to_doc(x, y)),
            _ => Aim::Own,
        };
        // The loops are capped here too: `GeometryOp` and `shape_path` are public, so `p` may not
        // have been through `clamped`.
        let how = match p.mode {
            DisplaceMode::Spatial => How::Spatial { scale: p.scale, octaves: p.octaves.min(MAX_OCTAVES), perm: perm_table(p.seed) },
            DisplaceMode::Radial => {
                let (phase_sin, phase_cos) = sin_cos(phase(p.seed, place));
                How::Radial { centre, ripples: p.ripple_count.min(MAX_RIPPLES), phase_cos, phase_sin, frame }
            }
            DisplaceMode::Wave => {
                // A wave's own direction is tlprocedural's default: across the width, moving in y.
                if aim == Aim::Own {
                    aim = Aim::Along(frame.to_doc(0.0, 1.0));
                }
                let across = match wave_along(p, frame) {
                    Some(along) => Across::Span { along, span: None },
                    None => Across::Outline,
                };
                let turns = if p.phase.is_finite() { p.phase / 360.0 } else { 0.0 };
                How::Wave { form: p.waveform, cycles: p.frequency, turns, across }
            }
            DisplaceMode::Jitter => How::Jitter { centre, least: p.jitter_min, state: Cell::new(jitter_start(p.seed, place)) },
            DisplaceMode::Fractal => How::Fractal(fractal::Grow::new(p, place, frame.flipped())),
        };
        Field { amplitude, how, aim, seen: Cell::new(0) }
    }

    /// A wave across the object's extent from `lo` to `hi` along [`wave_along`] (the anchors'
    /// projections); other patterns are as they were.
    pub fn spanning(mut self, lo: f64, hi: f64) -> Field {
        if let How::Wave { across: Across::Span { span, .. }, .. } = &mut self.how {
            let len = hi - lo;
            *span = (lo.is_finite() && len.is_finite() && len >= MIN_WAVE_SPAN).then_some((lo, len));
        }
        self
    }

    /// `sp` with every anchor moved (its handles with it), or grown (fractal); `None` when nothing
    /// changes.
    pub fn displace(&self, sp: &SubPath) -> Option<SubPath> {
        let before = self.seen.get();
        self.seen.set(before.saturating_add(sp.anchors.len()));
        if self.amplitude == 0.0 || !self.amplitude.is_finite() || sp.anchors.is_empty() {
            return None;
        }
        if let How::Fractal(g) = &self.how {
            return fractal::grow(sp, g, self.amplitude, self.aim, before);
        }
        let normals = if self.aim == Aim::Normal { normals(sp) } else { Vec::new() };
        let along = match &self.how {
            How::Wave { across: Across::Outline, .. } => Some(outline_fractions(sp)?),
            How::Wave { across: Across::Span { span: None, .. }, .. } => return None,
            _ => None,
        };
        let anchors = sp
            .anchors
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let dir = match self.aim {
                    Aim::Own => None,
                    Aim::Normal => Some(normals.get(i).copied().unwrap_or(Vec2::ZERO)),
                    Aim::Along(v) => Some(v),
                };
                let t = along.as_ref().and_then(|t| t.get(i).copied());
                let d = self.offset(a.p, dir, t);
                Anchor { p: a.p + d, h_in: a.h_in + d, h_out: a.h_out + d, kind: a.kind }
            })
            .collect();
        Some(SubPath::new(anchors, sp.closed))
    }

    /// How far the anchor at `p` moves: along `dir` when given (else the pattern's own way);
    /// `outline`: its fraction of its subpath's outline (a wave along it).
    fn offset(&self, p: Point, dir: Option<Vec2>, outline: Option<f64>) -> Vec2 {
        match &self.how {
            How::Spatial { scale, octaves, perm } => {
                let (x, y) = (p.x * scale, p.y * scale);
                let nx = fbm(x, y, *octaves, perm);
                if let Some(d) = dir {
                    return d * (nx * self.amplitude);
                }
                let ny = fbm(x + 100.0, y + 100.0, *octaves, perm);
                Vec2::new(nx * self.amplitude, ny * self.amplitude)
            }
            How::Radial { centre, ripples, phase_cos, phase_sin, frame } => {
                let v = p - *centre;
                // Plain √ (correctly rounded everywhere), not the platform's `hypot`.
                let dist = (v.x * v.x + v.y * v.y).sqrt();
                if dist <= 0.0 || !dist.is_finite() {
                    return Vec2::ZERO;
                }
                // sin(ripples·θ + phase), θ the anchor's angle round the centre in the copy's
                // frame: the unit vector raised to the ripples-th power (complex), turned by the
                // phase. The move itself is along `v`, in document space.
                let (lx, ly) = frame.local(v);
                let (c, s) = (lx / dist, ly / dist);
                let (mut re, mut im) = (1.0, 0.0);
                for _ in 0..*ripples {
                    (re, im) = (re * c - im * s, re * s + im * c);
                }
                let wave = im * phase_cos + re * phase_sin;
                // tlprocedural: bump = dist · amplitude/100 · 0.25 · wave, along the anchor's
                // direction from the centre.
                let k = self.amplitude / 100.0 * 0.25 * wave;
                match dir {
                    Some(d) => d * (dist * k),
                    None => v * k,
                }
            }
            How::Wave { form, cycles, turns, across } => {
                let t = match (across, outline) {
                    (Across::Outline, Some(t)) => t,
                    (Across::Span { along, span: Some((lo, len)) }, _) => (along.dot(p.to_vec2()) - lo) / len,
                    _ => return Vec2::ZERO,
                };
                let w = waveform(*form, t * cycles + turns);
                dir.map_or(Vec2::ZERO, |d| d * (self.amplitude * w))
            }
            How::Jitter { centre, least, state } => {
                let span = (1.0 - least).max(0.0);
                let amount = || {
                    // tlprocedural draws only when there is a range to draw from.
                    let u = if span > 0.0 { lcg_unit(state) } else { 0.0 };
                    self.amplitude * (least + u * span)
                };
                match dir {
                    Some(d) => d * amount(),
                    None => {
                        let v = p - *centre;
                        let len = (v.x * v.x + v.y * v.y).sqrt();
                        if !(len >= MIN_JITTER_LENGTH && len.is_finite()) {
                            return Vec2::ZERO;
                        }
                        // Along the anchor's direction from the centre, never through it.
                        let target = (len + amount()).max(MIN_JITTER_LENGTH);
                        v * (target / len - 1.0)
                    }
                }
            }
            How::Fractal(_) => Vec2::ZERO,
        }
    }
}

/// A wave's value at `x` cycles, -1..=1 (tlprocedural's `computeWave`). The square wave reads the
/// cycle's half directly (tlprocedural took the sine's sign, which is the same except exactly on
/// a half cycle, where its sine's last bits decided).
pub(super) fn waveform(form: Waveform, x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let m = ((x % 1.0) + 1.0) % 1.0;
    match form {
        // sin(2πx), read from the cycle's fraction (the same sine, without reducing a large angle).
        Waveform::Sine => sin_cos(m * std::f64::consts::TAU).0,
        Waveform::Square => {
            if m < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Waveform::Triangle => {
            if m < 0.25 {
                m * 4.0
            } else if m < 0.75 {
                2.0 - m * 4.0
            } else {
                m * 4.0 - 4.0
            }
        }
        Waveform::Sawtooth => m * 2.0 - 1.0,
    }
}

/// The jitter's first LCG state for `seed` at array place `place`: tlprocedural's `seed || 1` on
/// the object and the first copy; another copy's from the crate's seeded stream, so each copy
/// jitters its own way.
pub(super) fn jitter_start(seed: u32, place: usize) -> u64 {
    let s = if place == 0 {
        u64::from(seed)
    } else {
        let unit = (crate::kinds::rng_stream(seed, place).signed() + 1.0) / 2.0;
        (unit * LCG_MASK as f64) as u64 & LCG_MASK
    };
    if s == 0 { 1 } else { s }
}

/// The next of tlprocedural's `createSeededRandom` in 0..=1, in exact integer arithmetic.
fn lcg_unit(state: &Cell<u64>) -> f64 {
    let s = state.get().wrapping_mul(LCG_A).wrapping_add(LCG_C) & LCG_MASK;
    state.set(s);
    s as f64 / LCG_MASK as f64
}

/// The outward normal at each anchor of `sp` (zero where it has no direction): square to the
/// tangent (the handles' line, or the neighbours' chord at a corner), outward by the winding of
/// the anchors' polygon.
pub(super) fn normals(sp: &SubPath) -> Vec<Vec2> {
    let a = &sp.anchors;
    let n = a.len();
    let sign = if signed_area(sp) >= 0.0 { 1.0 } else { -1.0 };
    (0..n)
        .map(|i| {
            let Some(here) = a.get(i) else { return Vec2::ZERO };
            let handles = here.h_out - here.h_in;
            let t = if handles.x * handles.x + handles.y * handles.y > 1e-18 {
                handles
            } else {
                let prev = if i > 0 {
                    a.get(i - 1)
                } else if sp.closed {
                    a.last()
                } else {
                    None
                };
                let next = if i + 1 < n {
                    a.get(i + 1)
                } else if sp.closed {
                    a.first()
                } else {
                    None
                };
                next.map_or(here.p, |b| b.p) - prev.map_or(here.p, |b| b.p)
            };
            let len = (t.x * t.x + t.y * t.y).sqrt();
            if !(len > 1e-12 && len.is_finite()) {
                return Vec2::ZERO;
            }
            // The right-hand normal points out of a positively wound polygon.
            Vec2::new(t.y, -t.x) * (sign / len)
        })
        .collect()
}

/// Twice the signed area of `sp`'s anchor polygon (closed by its first anchor).
pub(super) fn signed_area(sp: &SubPath) -> f64 {
    let a = &sp.anchors;
    a.iter().zip(a.iter().cycle().skip(1)).map(|(p, q)| p.p.x * q.p.y - q.p.x * p.p.y).sum()
}

/// Each anchor's place along `sp`'s outline (the anchors' chords, a closed one's closing chord
/// too) as a fraction of it; `None` when it has no length.
fn outline_fractions(sp: &SubPath) -> Option<Vec<f64>> {
    let a = &sp.anchors;
    let mut at = Vec::with_capacity(a.len());
    let mut total = 0.0;
    for (i, x) in a.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).and_then(|j| a.get(j)) {
            let d = x.p - prev.p;
            total += (d.x * d.x + d.y * d.y).sqrt();
        }
        at.push(total);
    }
    if sp.closed
        && let (Some(first), Some(last)) = (a.first(), a.last())
    {
        let d = first.p - last.p;
        total += (d.x * d.x + d.y * d.y).sqrt();
    }
    if !(total >= MIN_WAVE_SPAN && total.is_finite()) {
        return None;
    }
    Some(at.into_iter().map(|s| s / total).collect())
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
/// bits on every platform (the platform's `sin` may differ in the last place). Accurate to the
/// last few bits for a phase or angle (|x| up to a few turns); larger `x` loses what `x % 2π`
/// loses.
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
