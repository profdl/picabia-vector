//! Displace's `fractal` pattern: subdivision growth, ported from tlprocedural's
//! `SubdivisionGrowthProcessor.subdividePolygon` (permission in NOTICE). Each step splits every
//! segment (at its middle, or a seeded place with `edgeVariance`) and pushes the new anchor out by
//! the amplitude, `decay` times less each step: away from the centre, along the segment's normal
//! or between them (`normalBias`), turned by a seeded angle (`wander`).
//!
//! Differences, on purpose: segments are split as curves (de Casteljau), so a curve stays a curve
//! and a straight segment stays straight until its new anchor moves (tlprocedural grew the
//! anchors' polygon); each subpath grows round its own centre (tlprocedural's centroid was the
//! whole shape's); an open path grows its segments only (tlprocedural outlined it first); the
//! anchors are never thinned (tlprocedural capped the source at 4096 / 2^steps points): a row that
//! would make more than the geometry cap is refused instead ([`count`] says how many, before
//! anything is allocated); and repulsion, intersection clean-up, growth bias and shape influence
//! are not ported (`docs/picabia/modifiers.md` › displace).

use vectorcraft_geom::{Anchor, AnchorKind, Point, SubPath, Vec2};

use super::MAX_GEOMETRY_ANCHORS;
use super::displace::{Aim, sin_cos};
use crate::model::Displace;

/// The most steps (tlprocedural's range).
pub(super) const MAX_STEPS: u32 = 8;

/// A fractal's params for one copy, read once.
pub(super) struct Grow {
    steps: u32,
    decay: f64,
    wander: f64,
    edge_variance: f64,
    normal_bias: f64,
    inward: bool,
    seed: u32,
    /// The copy's array place (tlprocedural's instance index).
    place: u32,
    /// The copy is reflected: its wander turns the other way, so a mirror pair stays a pair.
    flip: bool,
}

impl Grow {
    pub fn new(p: &Displace, place: usize, flip: bool) -> Grow {
        Grow {
            steps: p.steps.clamp(1, MAX_STEPS),
            decay: p.decay,
            wander: p.wander,
            edge_variance: p.edge_variance,
            normal_bias: p.normal_bias,
            inward: p.inward,
            seed: p.seed,
            place: u32::try_from(place).unwrap_or(u32::MAX),
            flip,
        }
    }
}

/// How many anchors `sp` has after `steps` steps (each doubles the segments), found without
/// growing it.
pub(super) fn count(sp: &SubPath, steps: u32) -> usize {
    let n = sp.anchors.len();
    let times = 1usize << steps.clamp(1, MAX_STEPS);
    if sp.closed && n >= 3 {
        n.saturating_mul(times)
    } else if !sp.closed && n >= 2 {
        (n - 1).saturating_mul(times).saturating_add(1)
    } else {
        n
    }
}

/// `sp` grown by `g` with push `amplitude` along `aim`; `before`: the object's anchors in the
/// subpaths before this one (the segments' seeds continue across them). `None`: too few anchors,
/// a closed subpath with no area (tlprocedural left it alone), or more anchors than the cap.
pub(super) fn grow(sp: &SubPath, g: &Grow, amplitude: f64, aim: Aim, before: usize) -> Option<SubPath> {
    if (sp.closed && sp.anchors.len() < 3) || sp.anchors.len() < 2 || count(sp, g.steps) > MAX_GEOMETRY_ANCHORS {
        return None;
    }
    let sign = if g.inward { -1.0 } else { 1.0 };
    let mut anchors = sp.anchors.clone();
    // decay^gen by repeated products (no platform `powf`).
    let mut share = 1.0;
    let mut offset = before;
    for gen_ in 0..g.steps {
        let strength = amplitude * share * sign;
        share *= g.decay;
        anchors = generation(&anchors, sp.closed, strength, (g, aim), gen_, offset)?;
        offset = offset.saturating_mul(2);
    }
    Some(SubPath::new(anchors, sp.closed))
}

/// tlprocedural's `seededRandom` (mulberry32 of `seed ^ counter·2654435761`), in 0..1. Its
/// arithmetic is 32-bit, so wrapping `u32` gives the same bits.
pub(super) fn seeded(seed: u32, counter: u32) -> f64 {
    let mut t = seed ^ counter.wrapping_mul(2_654_435_761);
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    f64::from(t ^ (t >> 14)) / 4_294_967_296.0
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    a + (b - a) * t
}

fn unit(v: Vec2, min: f64) -> Option<Vec2> {
    let len = (v.x * v.x + v.y * v.y).sqrt();
    (len > min && len.is_finite()).then(|| v / len)
}

/// One step: every segment of `a` split once, its new anchor pushed by `strength`.
fn generation(a: &[Anchor], closed: bool, strength: f64, (g, aim): (&Grow, Aim), gen_: u32, offset: usize) -> Option<Vec<Anchor>> {
    let len = a.len();
    let edges = if closed { len } else { len - 1 };
    let next = |i: usize| a.get((i + 1) % len);
    // tlprocedural's area-weighted centroid; a closed subpath with no area is left alone.
    let (mut cx, mut cy, mut area) = (0.0, 0.0, 0.0);
    for (i, p) in a.iter().enumerate() {
        let q = next(i)?;
        let cross = p.p.x * q.p.y - q.p.x * p.p.y;
        cx += (p.p.x + q.p.x) * cross;
        cy += (p.p.y + q.p.y) * cross;
        area += cross;
    }
    let outward_sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let centre = if area.abs() < 0.0001 {
        if closed {
            return None;
        }
        // An open path with no area: the mean of its anchors.
        let s = a.iter().fold(Vec2::ZERO, |s, x| s + x.p.to_vec2());
        (s / len as f64).to_point()
    } else {
        let half = area * 0.5;
        Point::new(cx / (6.0 * half), cy / (6.0 * half))
    };
    let mut total = 0.0;
    for i in 0..edges {
        let e = next(i)?.p - a.get(i)?.p;
        total += (e.x * e.x + e.y * e.y).sqrt();
    }
    // Segments shorter than this share of the average are split but not pushed (tlprocedural:
    // their spikes went every way).
    let min_len = if edges > 0 { total / edges as f64 } else { 1.0 } * 0.15;
    let mix = g.seed.wrapping_add(g.place.wrapping_mul(997)).wrapping_add(gen_.wrapping_mul(99_991));
    let mut out: Vec<Anchor> = Vec::with_capacity(len + edges);
    out.push(*a.first()?);
    for i in 0..edges {
        let b = *next(i)?;
        let here = *out.last()?;
        let edge_seed = (offset.wrapping_add(i) & 0xffff_ffff) as u32;
        let e = b.p - here.p;
        let elen = (e.x * e.x + e.y * e.y).sqrt();
        let pushed = elen >= min_len;
        let mut t = 0.5;
        if pushed && g.edge_variance > 0.0 {
            let r = seeded(mix, edge_seed.wrapping_mul(2));
            t = (0.5 + (r - 0.5) * g.edge_variance * 0.8).clamp(0.1, 0.9);
        }
        let straight = here.h_out == here.p && b.h_in == b.p;
        let (a_out, m_in, m, m_out, b_in) = if straight {
            let m = lerp(here.p, b.p, t);
            (here.h_out, m, m, m, b.h_in)
        } else {
            let (q0, q1, q2) = (lerp(here.p, here.h_out, t), lerp(here.h_out, b.h_in, t), lerp(b.h_in, b.p, t));
            let (r0, r1) = (lerp(q0, q1, t), lerp(q1, q2, t));
            (q0, r0, lerp(r0, r1, t), r1, q2)
        };
        let push = if pushed {
            let split = Split { m, centre, e, elen, outward_sign, mix, edge_seed };
            direction(&split, g, aim) * strength
        } else {
            Vec2::ZERO
        };
        if let Some(last) = out.last_mut() {
            last.h_out = a_out;
        }
        let kind = if straight { AnchorKind::Corner } else { AnchorKind::Smooth };
        out.push(Anchor { p: m + push, h_in: m_in + push, h_out: m_out + push, kind });
        if i + 1 < len {
            out.push(Anchor { h_in: b_in, ..b });
        } else if let Some(first) = out.first_mut() {
            first.h_in = b_in;
        }
    }
    Some(out)
}

/// Where a segment's new anchor sits and what it reads.
struct Split {
    m: Point,
    centre: Point,
    /// The segment, and its length.
    e: Vec2,
    elen: f64,
    /// 1 for a positively wound polygon, else -1.
    outward_sign: f64,
    mix: u32,
    edge_seed: u32,
}

/// The unit direction the new anchor moves: tlprocedural's blend of the radial direction and the
/// segment's normal (own), the outward normal (normal), or the own direction's share along an
/// axis; then turned by the seeded wander.
fn direction(s: &Split, g: &Grow, aim: Aim) -> Vec2 {
    let radial = unit(s.m - s.centre, 0.001).unwrap_or(Vec2::new(0.0, 1.0));
    // tlprocedural's winding normal: the left normal of a positively wound polygon, which points
    // in; its own blend first turns it into the radial half-plane.
    let wound = if s.elen > 0.001 { Vec2::new(-s.e.y / s.elen, s.e.x / s.elen) * s.outward_sign } else { Vec2::new(0.0, 1.0) };
    let own = || {
        let n = if s.elen > 0.001 && wound.dot(radial) < 0.0 { -wound } else { wound };
        let d = radial * (1.0 - g.normal_bias) + n * g.normal_bias;
        unit(d, 0.001).unwrap_or(d)
    };
    let mut d = match aim {
        Aim::Own => own(),
        Aim::Normal => -wound,
        Aim::Along(axis) => axis * own().dot(axis),
    };
    if g.wander > 0.0 {
        let r = seeded(s.mix, s.edge_seed.wrapping_mul(2).wrapping_add(1));
        let mut angle = (r - 0.5) * g.wander * (std::f64::consts::PI / 2.0);
        if g.flip {
            angle = -angle;
        }
        let (sin, cos) = sin_cos(angle);
        d = Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
    }
    d
}
