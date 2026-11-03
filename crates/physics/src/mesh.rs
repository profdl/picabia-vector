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

use std::collections::BTreeMap;

use rapier2d::prelude::{SoftBodyBuilder, Vector};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::PT_PER_M;
use crate::shape::{BodyShape, clean_ring, convex_hull, is_simple, segments_touch, signed_area2};
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
const BALL_POINTS: u32 = 32;
/// A self-colliding strip's least spacing, in particle radii. rapier (without its FEM solver) only
/// spares a particle's direct neighbours from self-contact, so particles closer than that would
/// start inside each other's skin and push apart for ever: the cloth stretches and stands stiff.
pub const SELF_CONTACT_SPACING: f64 = 1.25;
/// Tries at a coarser spacing before giving up.
const MAX_TRIES: usize = 8;
/// The most separate outlines one cell body fills (the rest are left out, with a note).
pub const MAX_CELL_PIECES: usize = 64;

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
    let fields = soft.fields();
    let spacing = fields.params().spacing;
    if soft.preset.is_strip() { strip(shape, spacing, fields.self_collide, cap) } else { cells(shape, spacing, cap) }
}

/// Every point of the outline (balls and boxes as points on their edges).
fn outline_points(shape: &BodyShape) -> Vec<Point> {
    shape.all_points().into_iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect()
}

fn strip(shape: &BodyShape, spacing: f64, self_collide: bool, cap: usize) -> Result<SoftMesh, String> {
    let pts = outline_points(shape);
    let first = *pts.first().ok_or("it has no outline")?;
    // The principal axis of the outline as drawn: its edges' midpoints weighted by their lengths,
    // so how finely a curve was flattened doesn't turn the axis.
    let mut weighted: Vec<(Point, f64)> = Vec::new();
    for piece in &shape.pieces {
        let ring = piece.outline(BALL_POINTS);
        let n = ring.len();
        let edges = if piece.is_closed() { n } else { n.saturating_sub(1) };
        for i in 0..edges {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n.max(1))) else { continue };
            let len = a.distance(b);
            if len.is_finite() && len > 0.0 {
                weighted.push((a.midpoint(b), len));
            }
        }
    }
    let total: f64 = weighted.iter().map(|(_, w)| w).sum();
    let (sxx, syy, sxy) = if total > 0.0 && total.is_finite() {
        let c = weighted.iter().fold(Vec2::ZERO, |a, (p, w)| a + p.to_vec2() * *w) / total;
        weighted.iter().fold((0.0, 0.0, 0.0), |(xx, yy, xy), (p, w)| {
            let d = p.to_vec2() - c;
            (xx + w * d.x * d.x, yy + w * d.y * d.y, xy + w * d.x * d.y)
        })
    } else {
        (0.0, 0.0, 0.0)
    };
    // Nearly round (no long axis): the bounds' longer side.
    let spread = ((sxx - syy) * (sxx - syy) + 4.0 * sxy * sxy).sqrt();
    let axis = if !spread.is_finite() || spread <= 0.02 * (sxx + syy) {
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
    let least = SELF_CONTACT_SPACING * thickness / 2.0;
    let raised = self_collide && spacing < least;
    if raised {
        notes.push(format!("self-collide: spacing at least {least:.1} pt, so the strip's {thickness:.1} pt thickness doesn't push it apart"));
    }
    // Raised: rounded down, so the spacing used never goes back under the least.
    let steps = length / if raised { least } else { spacing };
    let wanted = (if raised { steps.floor() } else { steps.round() }) as usize + 1;
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

/// The closed outlines to fill, each cleaned and simple (a crossing one is its hull), with its
/// outline group ([`BodyShape::owners`]). Outlines of different groups that overlap, or rings of
/// one group that cross each other, are filled together as their hull (with a note): parry would
/// drop crossing edges and leave overlapping particles at rest.
fn rings_of(shape: &BodyShape, notes: &mut Vec<String>) -> Vec<(Vec<Point>, usize)> {
    let mut out: Vec<(Vec<Point>, usize)> = Vec::new();
    let mut hulled = 0;
    for (i, piece) in shape.pieces.iter().enumerate() {
        if !piece.is_closed() {
            continue;
        }
        let mut ring = clean_ring(&piece.outline(BALL_POINTS));
        if ring.len() < 3 || signed_area2(&ring).abs() < 1e-6 {
            continue;
        }
        if !is_simple(&ring) {
            ring = convex_hull(&ring);
            hulled += 1;
        }
        if ring.len() >= 3 {
            out.push((ring, shape.owners.get(i).copied().unwrap_or(0)));
        }
    }
    if hulled > 0 {
        notes.push(plural(hulled, "outline crosses itself: filled as its hull", "outlines cross themselves: filled as their hulls"));
    }
    // Merge what overlaps across groups, or crosses within one, into hulls.
    let mut merged = 0;
    loop {
        let mut pair = None;
        'find: for i in 0..out.len() {
            for j in i + 1..out.len() {
                let (Some((a, ga)), Some((b, gb))) = (out.get(i), out.get(j)) else { continue };
                let nested = a.first().is_some_and(|p| inside(b, *p)) || b.first().is_some_and(|p| inside(a, *p));
                if rings_cross(a, b) || (ga != gb && nested) {
                    pair = Some((i, j));
                    break 'find;
                }
            }
        }
        let Some((i, j)) = pair else { break };
        let b = out.swap_remove(j);
        if let Some(a) = out.get_mut(i) {
            let mut all = std::mem::take(&mut a.0);
            all.extend(b.0);
            a.0 = convex_hull(&all);
        }
        merged += 1;
    }
    if merged > 0 {
        notes.push(plural(merged, "overlapping outline filled with its neighbour as their hull", "overlapping outlines filled as their hulls"));
    }
    out.retain(|(r, _)| r.len() >= 3);
    out
}

/// Whether two rings' edges cross or touch.
fn rings_cross(a: &[Point], b: &[Point]) -> bool {
    let (na, nb) = (a.len(), b.len());
    let bounds = |r: &[Point]| r.iter().fold(Rect::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |r, p| r.union_pt(*p));
    let (ra, rb) = (bounds(a), bounds(b));
    if ra.x1 < rb.x0 || rb.x1 < ra.x0 || ra.y1 < rb.y0 || rb.y1 < ra.y0 {
        return false;
    }
    for i in 0..na {
        let (Some(&p), Some(&q)) = (a.get(i), a.get((i + 1) % na)) else { continue };
        for j in 0..nb {
            let (Some(&r), Some(&t)) = (b.get(j), b.get((j + 1) % nb)) else { continue };
            if segments_touch(p, q, r, t) {
                return true;
            }
        }
    }
    false
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Whether `p` is inside ring `r` (even-odd).
fn inside(r: &[Point], p: Point) -> bool {
    let mut on = false;
    let n = r.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (r.get(i), r.get((i + n - 1) % n)) else { continue };
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            on = !on;
        }
    }
    on
}

/// The length of closed ring `r`.
fn ring_length(r: &[Point]) -> f64 {
    let n = r.len();
    (0..n).filter_map(|i| Some(r.get(i)?.distance(*r.get((i + 1) % n)?))).sum()
}

/// `ring` with each edge split into pieces at most about `spacing` long; its corners are kept.
fn resample(ring: &[Point], spacing: f64) -> Vec<Point> {
    let n = ring.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
        let k = (a.distance(b) / spacing).ceil();
        let k = if k.is_finite() { (k as usize).clamp(1, MAX_PARTICLES_PER_BODY) } else { 1 };
        out.extend((0..k).map(|j| a.lerp(b, j as f64 / k as f64)));
    }
    out
}

fn cells(shape: &BodyShape, spacing: f64, cap: usize) -> Result<SoftMesh, String> {
    let mut notes = Vec::new();
    let mut rings = rings_of(shape, &mut notes);
    if rings.is_empty() {
        return Err("a jelly needs a closed outline with some area".into());
    }
    // Holes: an outline inside another of its group (even-odd depth within the group).
    let depth = |rings: &[(Vec<Point>, usize)], i: usize| -> usize {
        let Some((r, g)) = rings.get(i) else { return 0 };
        rings.iter().enumerate().filter(|(j, (o, og))| *j != i && og == g && r.first().is_some_and(|p| inside(o, *p))).count()
    };
    let area: f64 = (0..rings.len())
        .map(|i| if depth(&rings, i) % 2 == 0 { 1.0 } else { -1.0 } * rings.get(i).map_or(0.0, |(r, _)| signed_area2(r).abs() / 2.0))
        .sum();
    let perimeter: f64 = rings.iter().map(|(r, _)| ring_length(r)).sum();
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
    // Outlines smaller than a cell can't be filled: outer ones are left out, holes filled in.
    let before = rings.len();
    let tiny = |r: &[Point], h: f64| signed_area2(r).abs() / 2.0 < h * h * 0.25;
    let keep: Vec<bool> = rings.iter().map(|(r, _)| !tiny(r, h)).collect();
    let mut k = 0;
    rings.retain(|_| {
        k += 1;
        keep.get(k - 1).copied().unwrap_or(false)
    });
    let dropped = before - rings.len();
    if dropped > 0 {
        notes.push(plural(dropped, "outline smaller than a cell left out", "outlines smaller than a cell left out"));
    }
    if rings.is_empty() {
        return Err("its outlines are all smaller than a cell".into());
    }
    let depth: Vec<usize> = (0..rings.len()).map(|i| depth(&rings, i)).collect();
    let outer: Vec<usize> = (0..rings.len()).filter(|i| depth.get(*i).is_some_and(|d| d % 2 == 0)).collect();
    if outer.len() > MAX_CELL_PIECES {
        notes.push(format!("{} separate outlines (at most {MAX_CELL_PIECES}): the rest left out", outer.len()));
    }
    let origin = shape.bounds.center();
    let to_m = |p: Point| Vector::new(((p.x - origin.x) / PT_PER_M) as f32, ((p.y - origin.y) / PT_PER_M) as f32);
    for _ in 0..MAX_TRIES {
        let mut body: Option<SoftBodyBuilder> = None;
        let mut unfilled = 0;
        let mut over = false;
        for &i in outer.iter().take(MAX_CELL_PIECES) {
            let (Some((r, g)), Some(&d)) = (rings.get(i), depth.get(i)) else { continue };
            // This outline and the holes directly inside it.
            let mut verts: Vec<Vector> = Vec::new();
            let mut segs: Vec<[u32; 2]> = Vec::new();
            let mut add = |ring: &[Point]| {
                let s = resample(ring, h);
                let base = verts.len() as u32;
                let k = s.len() as u32;
                verts.extend(s.iter().map(|p| to_m(*p)));
                segs.extend((0..k).map(|j| [base + j, base + (j + 1) % k]));
            };
            add(r);
            for (j, (hole, hg)) in rings.iter().enumerate() {
                if hg == g && depth.get(j) == Some(&(d + 1)) && hole.first().is_some_and(|p| inside(r, *p)) {
                    add(hole);
                }
            }
            if verts.len() > cap {
                over = true;
                break;
            }
            match SoftBodyBuilder::volumetric(&verts, &segs, (h / PT_PER_M) as f32) {
                Some(p) => {
                    let b = match body.take() {
                        Some(b) => b.append(p),
                        None => p,
                    };
                    // Over the caps already: try again coarser without meshing the rest.
                    if b.positions.len() > cap || b.cells.len() > MAX_CELLS_PER_BODY {
                        over = true;
                        break;
                    }
                    body = Some(b);
                }
                None => unfilled += 1,
            }
        }
        if over {
            h *= 1.25;
            continue;
        }
        let Some(b) = body else {
            return Err("its outline couldn't be filled with cells".into());
        };
        if unfilled > 0 {
            notes.push(plural(unfilled, "outline couldn't be filled with cells: left out", "outlines couldn't be filled with cells: left out"));
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
    let mut count: BTreeMap<[u32; 2], (u32, [u32; 2])> = BTreeMap::new();
    for c in cells {
        for k in 0..3 {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            let e = count.entry([a.min(b), a.max(b)]).or_insert((0, [a, b]));
            e.0 += 1;
        }
    }
    count.into_values().filter(|(n, _)| *n == 1).map(|(_, e)| e).collect()
}
