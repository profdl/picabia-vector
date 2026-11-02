//! Soft bodies from objects (PIC31.2): an object's outline to a rest mesh, the particles and cells
//! (or strip) rapier simulates, in document space.
//!
//! - **Cloth**: the outline's long axis (its principal axis, so it turns with the object) is the
//!   strip, its extent across the thickness: a line of particles along the centre, `spacing` apart.
//! - **Jelly / rubber**: the closed outlines (cleaned, resampled to `spacing`) filled with triangle
//!   cells by rapier's `volumetric` (parry's Delaunay refinement); holes stay holes, separate
//!   outlines are separate pieces of the one body.
//!
//! Caps: [`MAX_PARTICLES_PER_BODY`] and [`MAX_CELLS_PER_BODY`] per object, and the caller's budget
//! (the run's particles left): a body that would go over gets a coarser spacing, with a note.
//! Input is validated before rapier sees it (PIC31.0: rapier panics on short or non-finite
//! outlines and has no cap on the cells it makes): finite points, rings of at least 3 points with
//! area, no crossing edges (a crossing ring is its hull), a cell size estimated against the caps
//! before meshing and the result checked after.

use rapier2d::prelude::{SoftBodyBuilder, Vector};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::PT_PER_M;
use crate::shape::{BodyShape, Piece, clean_ring, convex_hull, is_simple, signed_area2};
use crate::soft::Soft;

/// The most particles one soft body has (more: a coarser spacing, with a note).
pub const MAX_PARTICLES_PER_BODY: usize = 1_200;
/// The most cells one soft body has (more: a coarser spacing, with a note).
pub const MAX_CELLS_PER_BODY: usize = 2_400;
/// The most particles in one run (soft bodies past it are left out, with a note).
pub const MAX_PARTICLES_PER_RUN: usize = 12_000;
/// The thinnest strip, points (a hairline still has a contact skin).
pub const MIN_THICKNESS: f64 = 0.5;
/// Points per ring a ball becomes.
const BALL_POINTS: usize = 32;
/// Tries at a coarser spacing before giving up.
const MAX_TRIES: usize = 8;

/// A cloth strip's frame, document space.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// The centre line's start (the end where `along` is 0).
    pub origin: Point,
    /// Unit vector along the strip.
    pub axis: Vec2,
    /// The strip's length along `axis`, points.
    pub length: f64,
    /// Its extent across, points.
    pub thickness: f64,
}

impl Strip {
    /// Unit vector across the strip (the axis turned a quarter clockwise on the page).
    pub fn normal(&self) -> Vec2 {
        Vec2::new(-self.axis.y, self.axis.x)
    }

    /// `p` as (distance along the axis from `origin`, offset across it).
    pub fn local(&self, p: Point) -> (f64, f64) {
        let d = p - self.origin;
        (d.dot(self.axis), d.dot(self.normal()))
    }
}

/// How the rest mesh is laid out.
#[derive(Clone, Debug, PartialEq)]
pub enum MeshKind {
    /// Cloth: particles along the strip's centre line, in order.
    Strip(Strip),
    /// Jelly, rubber: particles filling the outline, joined by triangle cells.
    Cells,
}

/// A soft body's rest mesh, document space.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftMesh {
    pub kind: MeshKind,
    /// The particles' rest positions.
    pub rest: Vec<Point>,
    /// Triangle cells (indices into `rest`): empty for a strip.
    pub cells: Vec<[u32; 3]>,
    /// The strip's segments, or the cells' boundary.
    pub segments: Vec<[u32; 2]>,
    /// Particle radius, points (the contact skin; half the strip's thickness for cloth).
    pub radius: f64,
    /// The spacing used, points (coarser than asked when a cap was reached).
    pub spacing: f64,
    pub notes: Vec<String>,
}

impl SoftMesh {
    /// The rest particles' bounds.
    pub fn bounds(&self) -> Option<Rect> {
        let first = *self.rest.first()?;
        Some(self.rest.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)))
    }

    /// The index of the particle nearest `p`.
    pub fn nearest(&self, p: Point) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, q) in self.rest.iter().enumerate() {
            let d = (*q - p).hypot2();
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }
}

/// The rest mesh of an object whose outline is `shape`, with `soft`'s preset and fields, within
/// `budget` particles (at most [`MAX_PARTICLES_PER_BODY`]); or why it has none.
pub fn soft_mesh(shape: &BodyShape, soft: &Soft, budget: usize) -> Result<SoftMesh, String> {
    let cap = budget.min(MAX_PARTICLES_PER_BODY);
    if cap < 2 {
        return Err(format!("over the {MAX_PARTICLES_PER_RUN}-particle limit for soft bodies in one run"));
    }
    let spacing = soft.fields().params().spacing;
    if soft.preset.is_strip() { strip(shape, spacing, cap) } else { cells(shape, spacing, cap) }
}

/// Every point of the outline (balls and boxes as points on their edges).
fn outline_points(shape: &BodyShape) -> Vec<Point> {
    shape.all_points().into_iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect()
}

fn strip(shape: &BodyShape, spacing: f64, cap: usize) -> Result<SoftMesh, String> {
    let pts = outline_points(shape);
    let first = *pts.first().ok_or("it has no outline")?;
    // The principal axis: the eigenvector of the points' covariance with the larger eigenvalue.
    let n = pts.len() as f64;
    let c = pts.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / n;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in &pts {
        let d = p.to_vec2() - c;
        sxx += d.x * d.x;
        syy += d.y * d.y;
        sxy += d.x * d.y;
    }
    // Nearly round (no long axis): the bounds' longer side.
    let spread = ((sxx - syy) * (sxx - syy) + 4.0 * sxy * sxy).sqrt();
    let axis = if spread <= 0.02 * (sxx + syy) || !spread.is_finite() {
        if shape.bounds.width() >= shape.bounds.height() { Vec2::new(1.0, 0.0) } else { Vec2::new(0.0, 1.0) }
    } else {
        let a = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let v = Vec2::new(a.cos(), a.sin());
        // Left to right (or top to bottom) on the page, so the strip's start is predictable.
        if v.x < -1e-9 || (v.x.abs() <= 1e-9 && v.y < 0.0) { -v } else { v }
    };
    let normal = Vec2::new(-axis.y, axis.x);
    let (mut s0, mut s1, mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        let d = *p - first;
        let (s, t) = (d.dot(axis), d.dot(normal));
        (s0, s1, t0, t1) = (s0.min(s), s1.max(s), t0.min(t), t1.max(t));
    }
    let length = s1 - s0;
    if !(length.is_finite() && length >= crate::shape::MIN_SIZE) {
        return Err("it has no length to make a strip of".into());
    }
    let thickness = (t1 - t0).max(MIN_THICKNESS);
    let origin = first + axis * s0 + normal * ((t0 + t1) / 2.0);
    let mut notes = Vec::new();
    let wanted = (length / spacing).round() as usize + 1;
    let count = wanted.clamp(2, cap.max(2));
    if count < wanted {
        notes.push(format!("{wanted} particles (at most {cap}): spacing {:.1} pt", length / (count - 1) as f64));
    }
    let used = length / (count - 1) as f64;
    let rest: Vec<Point> = (0..count).map(|i| origin + axis * (used * i as f64)).collect();
    let segments = (0..count as u32 - 1).map(|i| [i, i + 1]).collect();
    Ok(SoftMesh {
        kind: MeshKind::Strip(Strip { origin, axis, length, thickness }),
        rest,
        cells: Vec::new(),
        segments,
        radius: thickness / 2.0,
        spacing: used,
        notes,
    })
}

/// One closed outline, cleaned and simple (a crossing one is its hull), counter-clockwise on the
/// page (positive [`signed_area2`] in document axes).
fn rings_of(shape: &BodyShape, notes: &mut Vec<String>) -> Vec<Vec<Point>> {
    let mut out = Vec::new();
    let mut hulled = 0;
    for piece in &shape.pieces {
        let ring: Vec<Point> = match piece {
            Piece::Ring { pts, closed: true } => pts.clone(),
            Piece::Ring { closed: false, .. } => continue,
            Piece::Ball { center, radius } => (0..BALL_POINTS)
                .map(|i| i as f64 * std::f64::consts::TAU / BALL_POINTS as f64)
                .map(|a| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin()))
                .collect(),
            Piece::Box(r) => vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)],
        };
        let mut ring = clean_ring(&ring);
        if ring.len() < 3 || signed_area2(&ring).abs() < 1e-6 {
            continue;
        }
        if !is_simple(&ring) {
            ring = convex_hull(&ring);
            hulled += 1;
        }
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    if hulled > 0 {
        notes.push(format!("{hulled} outlines cross themselves: filled as their hulls"));
    }
    out
}

/// Whether `p` is inside ring `r` (even-odd).
fn inside(r: &[Point], p: Point) -> bool {
    let mut on = false;
    let n = r.len();
    for i in 0..n {
        let (a, b) = (r[i], r[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            on = !on;
        }
    }
    on
}

/// `ring` resampled to points `spacing` apart along it (at least 3 per ring, its corners kept when
/// they fall between samples is not needed: the cells follow the samples).
fn resample(ring: &[Point], spacing: f64) -> Vec<Point> {
    let n = ring.len();
    let per: f64 = (0..n).map(|i| ring[i].distance(ring[(i + 1) % n])).sum();
    let count = ((per / spacing).round() as usize).max(3);
    if count >= n && n >= 3 && per / n as f64 <= spacing * 1.5 {
        return ring.to_vec();
    }
    let step = per / count as f64;
    let mut out = Vec::with_capacity(count);
    let mut seg = 0usize;
    let mut seg_start = 0.0;
    for k in 0..count {
        let at = k as f64 * step;
        while seg + 1 < n && seg_start + ring[seg].distance(ring[(seg + 1) % n]) < at {
            seg_start += ring[seg].distance(ring[(seg + 1) % n]);
            seg += 1;
        }
        let (a, b) = (ring[seg], ring[(seg + 1) % n]);
        let len = a.distance(b);
        let t = if len > 0.0 { ((at - seg_start) / len).clamp(0.0, 1.0) } else { 0.0 };
        out.push(a.lerp(b, t));
    }
    out
}

fn cells(shape: &BodyShape, spacing: f64, cap: usize) -> Result<SoftMesh, String> {
    let mut notes = Vec::new();
    let rings = rings_of(shape, &mut notes);
    if rings.is_empty() {
        return Err("a jelly needs a closed outline with some area".into());
    }
    // Outer outlines (even depth) with the holes directly inside them (odd depth).
    let depth: Vec<usize> = rings
        .iter()
        .enumerate()
        .map(|(i, r)| rings.iter().enumerate().filter(|(j, o)| *j != i && r.first().is_some_and(|p| inside(o, *p))).count())
        .collect();
    let area: f64 = rings.iter().zip(&depth).map(|(r, d)| if d % 2 == 0 { 1.0 } else { -1.0 } * signed_area2(r).abs() / 2.0).sum();
    let perimeter: f64 = rings.iter().map(|r| (0..r.len()).map(|i| r[i].distance(r[(i + 1) % r.len()])).sum::<f64>()).sum();
    if !(area.is_finite() && area > 0.0) {
        return Err("a jelly needs a closed outline with some area".into());
    }
    // Estimated particles at a spacing: the boundary's and the inside's (equilateral cells).
    let estimate = |h: f64| perimeter / h + area / (h * h * 0.866);
    let mut h = spacing;
    let wanted = estimate(spacing);
    while estimate(h) > cap as f64 || 2.0 * estimate(h) > MAX_CELLS_PER_BODY as f64 {
        h *= 1.1;
    }
    let origin = shape.bounds.center();
    let to_m = |p: Point| Vector::new(((p.x - origin.x) / PT_PER_M) as f32, ((p.y - origin.y) / PT_PER_M) as f32);
    for _ in 0..MAX_TRIES {
        let mut body: Option<SoftBodyBuilder> = None;
        for (i, r) in rings.iter().enumerate() {
            if depth[i] % 2 == 1 {
                continue;
            }
            // This outline and the holes directly inside it.
            let mut verts: Vec<Vector> = Vec::new();
            let mut segs: Vec<[u32; 2]> = Vec::new();
            let mut add = |ring: &[Point], ccw: bool| {
                let mut s = resample(ring, h);
                if (signed_area2(&s) > 0.0) != ccw {
                    s.reverse();
                }
                let base = verts.len() as u32;
                let k = s.len() as u32;
                verts.extend(s.iter().map(|p| to_m(*p)));
                segs.extend((0..k).map(|j| [base + j, base + (j + 1) % k]));
            };
            add(r, true);
            for (j, hole) in rings.iter().enumerate() {
                if depth[j] == depth[i] + 1 && hole.first().is_some_and(|p| inside(r, *p)) {
                    add(hole, false);
                }
            }
            let cell = (h / PT_PER_M) as f32;
            let piece = SoftBodyBuilder::volumetric(&verts, &segs, cell).or_else(|| {
                let flipped: Vec<[u32; 2]> = segs.iter().map(|[a, b]| [*b, *a]).collect();
                SoftBodyBuilder::volumetric(&verts, &flipped, cell)
            });
            if let Some(p) = piece {
                body = Some(match body {
                    Some(b) => b.append(p),
                    None => p,
                });
            }
        }
        let Some(b) = body else {
            return Err("its outline couldn't be filled with cells".into());
        };
        if b.positions.len() > cap || b.cells.len() > MAX_CELLS_PER_BODY {
            h *= 1.25;
            continue;
        }
        if h > spacing * 1.001 {
            notes.push(format!("about {wanted:.0} particles at {spacing:.1} pt (at most {cap}): spacing {h:.1} pt"));
        }
        let rest: Vec<Point> =
            b.positions.iter().map(|v| Point::new(origin.x + f64::from(v.x) * PT_PER_M, origin.y + f64::from(v.y) * PT_PER_M)).collect();
        let segments = boundary(&b.cells);
        return Ok(SoftMesh {
            kind: MeshKind::Cells,
            rest,
            cells: b.cells.clone(),
            segments,
            radius: (f64::from(b.particle_radius) * PT_PER_M).max(MIN_THICKNESS / 2.0),
            spacing: h,
            notes,
        });
    }
    Err(format!("its outline needs more than {cap} particles even at a coarse spacing"))
}

/// The edges of `cells` used by one cell only (the outline and the holes).
fn boundary(cells: &[[u32; 3]]) -> Vec<[u32; 2]> {
    let mut count: std::collections::BTreeMap<[u32; 2], (u32, [u32; 2])> = std::collections::BTreeMap::new();
    for c in cells {
        for k in 0..3 {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            let e = count.entry([a.min(b), a.max(b)]).or_insert((0, [a, b]));
            e.0 += 1;
        }
    }
    count.into_values().filter(|(n, _)| *n == 1).map(|(_, e)| e).collect()
}
