//! Pose to art (PIC31.3): a body's simulated state carried onto its object.
//!
//! - **Rigid**: a document-space affine (a turn about the bounds' centre and a move), which the
//!   engine applies with the transform commands' own path.
//! - **Cloth**: each point placed by its distance along the strip's axis and its offset across it,
//!   on the simulated centre line (the direction across follows the line's turning smoothly).
//! - **Jelly / rubber**: each point carried by its rest cell (barycentric) to the cell's simulated
//!   triangle; a point outside every cell follows the nearest one. Curves are split into short
//!   pieces first (`vectorcraft_doc::live::map_nonlinear_within`, as Deform does), so straight
//!   sides bend.
//!
//! Paths bend (a live shape becomes a plain path); type, images, symbol instances and other kinds
//! that can't bend follow the warp's best-fit affine at their centre, and linear gradients follow
//! the warp where they sit.

use std::sync::Arc;

use vectorcraft_doc::live::{affine_near, map_nonlinear_within};
use vectorcraft_doc::{Node, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::mesh::{MeshKind, SoftMesh, Strip};

/// The most anchors one warped path may have (pieces grow beyond it).
pub const MAX_WARPED_ANCHORS: usize = 50_000;

/// A body's state at one moment.
#[derive(Clone, Debug, PartialEq)]
pub enum Pose {
    /// A rigid body's motion from its rest place (`None`: it hasn't moved).
    Rigid(Option<Affine>),
    /// A soft body's particles, document space, in the mesh's order.
    Soft(Vec<Point>),
}

/// `node` warped from `mesh`'s rest pose to `particles` (document space, one per rest particle),
/// or `None` when the particles don't match the mesh or aren't finite.
pub fn pose_node(node: &Node, mesh: &SoftMesh, particles: &[Point]) -> Option<Node> {
    let w = Warp::new(mesh, particles)?;
    let piece = (mesh.spacing / 2.0).clamp(0.5, 50.0);
    let mut out = node.clone();
    warp_node(&mut out, &w, piece, 0);
    Some(out)
}

/// The point map of a pose.
pub struct Warp<'a> {
    mesh: &'a SoftMesh,
    to: &'a [Point],
    grid: Option<Grid>,
}

impl<'a> Warp<'a> {
    /// The map from `mesh`'s rest pose to `to`, or `None` when they don't match or aren't finite.
    pub fn new(mesh: &'a SoftMesh, to: &'a [Point]) -> Option<Warp<'a>> {
        if to.len() != mesh.rest.len() || to.is_empty() || !to.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        let grid = match mesh.kind {
            MeshKind::Cells => Some(Grid::new(mesh)?),
            MeshKind::Strip(_) => None,
        };
        Some(Warp { mesh, to, grid })
    }

    /// Where rest point `p` goes.
    pub fn at(&self, p: Point) -> Point {
        let q = match &self.mesh.kind {
            MeshKind::Strip(s) => self.along(s, p),
            MeshKind::Cells => self.in_cells(p),
        };
        if q.x.is_finite() && q.y.is_finite() { q } else { p }
    }

    fn particle(&self, i: usize) -> Point {
        self.to.get(i).copied().unwrap_or_default()
    }

    /// The simulated centre line's unit direction at particle `i` (central difference).
    fn tangent(&self, i: usize) -> Vec2 {
        let n = self.to.len();
        let (a, b) = (i.saturating_sub(1), (i + 1).min(n.saturating_sub(1)));
        let d = self.particle(b) - self.particle(a);
        let len = d.hypot();
        if len > 1e-12 { d / len } else { Vec2::new(1.0, 0.0) }
    }

    fn along(&self, s: &Strip, p: Point) -> Point {
        let n = self.to.len();
        if n < 2 {
            return p;
        }
        let (u, t) = s.local(p);
        let step = s.length / (n - 1) as f64;
        let x = if step > 0.0 { u / step } else { 0.0 };
        // Beyond the ends the end segment is extended.
        let k = (x.floor().max(0.0) as usize).min(n - 2);
        let f = x - k as f64;
        let (a, b) = (self.particle(k), self.particle(k + 1));
        let centre = a + (b - a) * f;
        // The direction turns smoothly between particles; across is a quarter turn of it, as the
        // strip's rest normal is of its axis.
        let fc = f.clamp(0.0, 1.0);
        let dir = self.tangent(k) * (1.0 - fc) + self.tangent(k + 1) * fc;
        let len = dir.hypot();
        let dir = if len > 1e-12 { dir / len } else { (b - a) / (b - a).hypot().max(1e-12) };
        centre + Vec2::new(-dir.y, dir.x) * t
    }

    fn in_cells(&self, p: Point) -> Point {
        let Some(g) = &self.grid else { return p };
        let Some((c, l)) = g.find(self.mesh, p) else { return p };
        let [a, b, cc] = c;
        let v = self.particle(a as usize).to_vec2() * l[0] + self.particle(b as usize).to_vec2() * l[1] + self.particle(cc as usize).to_vec2() * l[2];
        v.to_point()
    }
}

/// `p`'s barycentric coordinates in triangle `t`, `None` when it has no area.
fn bary(t: [Point; 3], p: Point) -> Option<[f64; 3]> {
    let (v0, v1, v2) = (t[1] - t[0], t[2] - t[0], p - t[0]);
    let d = v0.x * v1.y - v1.x * v0.y;
    if d.abs() < 1e-18 {
        return None;
    }
    let l1 = (v2.x * v1.y - v1.x * v2.y) / d;
    let l2 = (v0.x * v2.y - v2.x * v0.y) / d;
    Some([1.0 - l1 - l2, l1, l2])
}

/// The rest cells sorted into a uniform grid, for finding the cell under a point.
struct Grid {
    bounds: Rect,
    cell: f64,
    cols: usize,
    rows: usize,
    bins: Vec<Vec<u32>>,
}

impl Grid {
    fn new(mesh: &SoftMesh) -> Option<Grid> {
        let bounds = mesh.bounds()?;
        let n = mesh.cells.len().max(1);
        // About two cells a bin.
        let cell = ((bounds.width() * bounds.height() / n as f64 * 2.0).sqrt()).max(mesh.spacing.max(1e-3));
        let cols = ((bounds.width() / cell).ceil() as usize).clamp(1, 512);
        let rows = ((bounds.height() / cell).ceil() as usize).clamp(1, 512);
        let mut bins = vec![Vec::new(); cols * rows];
        let g0 = Grid { bounds, cell, cols, rows, bins: Vec::new() };
        for (i, c) in mesh.cells.iter().enumerate() {
            let pts = c.map(|k| mesh.rest.get(k as usize).copied().unwrap_or_default());
            let r = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
            let (c0, r0) = g0.bin(Point::new(r.x0, r.y0));
            let (c1, r1) = g0.bin(Point::new(r.x1, r.y1));
            for row in r0..=r1 {
                for col in c0..=c1 {
                    if let Some(b) = bins.get_mut(row * cols + col) {
                        b.push(i as u32);
                    }
                }
            }
        }
        Some(Grid { bins, ..g0 })
    }

    fn bin(&self, p: Point) -> (usize, usize) {
        let c = ((p.x - self.bounds.x0) / self.cell).floor();
        let r = ((p.y - self.bounds.y0) / self.cell).floor();
        let c = if c.is_finite() { c.clamp(0.0, (self.cols - 1) as f64) as usize } else { 0 };
        let r = if r.is_finite() { r.clamp(0.0, (self.rows - 1) as f64) as usize } else { 0 };
        (c, r)
    }

    /// The cell holding `p` with its barycentric coordinates; else the nearest cell (by its
    /// centroid among the bins around `p`, then all of them) with `p`'s coordinates in it.
    fn find(&self, mesh: &SoftMesh, p: Point) -> Option<([u32; 3], [f64; 3])> {
        let tri = |i: u32| -> Option<([u32; 3], [Point; 3])> {
            let c = *mesh.cells.get(i as usize)?;
            Some((c, c.map(|k| mesh.rest.get(k as usize).copied().unwrap_or_default())))
        };
        let (c, r) = self.bin(p);
        if let Some(bin) = self.bins.get(r * self.cols + c) {
            for &i in bin {
                if let Some((c, t)) = tri(i)
                    && let Some(l) = bary(t, p)
                    && l.iter().all(|x| *x >= -1e-9)
                {
                    return Some((c, l));
                }
            }
        }
        // Outside: the nearest cell by centroid, near first.
        let consider = |i: u32, best: &mut Option<(u32, f64)>| {
            if let Some((_, t)) = tri(i) {
                let cen = Point::new((t[0].x + t[1].x + t[2].x) / 3.0, (t[0].y + t[1].y + t[2].y) / 3.0);
                let d = (cen - p).hypot2();
                if best.is_none_or(|(_, b)| d < b) {
                    *best = Some((i, d));
                }
            }
        };
        let mut best: Option<(u32, f64)> = None;
        for row in r.saturating_sub(1)..=(r + 1).min(self.rows - 1) {
            for col in c.saturating_sub(1)..=(c + 1).min(self.cols - 1) {
                for &i in self.bins.get(row * self.cols + col).map_or(&[][..], |v| v.as_slice()) {
                    consider(i, &mut best);
                }
            }
        }
        if best.is_none() {
            for i in 0..mesh.cells.len() as u32 {
                consider(i, &mut best);
            }
        }
        let (i, _) = best?;
        let (c, t) = tri(i)?;
        Some((c, bary(t, p)?))
    }
}

/// Bend `n`'s paths through `w` (pieces of at most `piece` points); kinds that can't bend take the
/// warp's best-fit affine at their centre.
fn warp_node(n: &mut Node, w: &Warp, piece: f64, depth: usize) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    let f = |p: Point| w.at(p);
    if let Some(b) = n.geometric_bounds() {
        let eps = (b.width().max(b.height()) * 1e-3).max(1e-3);
        n.appearance.warp_linear_gradients(b, &|p| affine_near(&f, p, eps));
    }
    match &mut n.kind {
        NodeKind::Path { guide: true, .. } => {}
        NodeKind::Path { path, live, .. } => {
            *path = map_nonlinear_within(path, piece, MAX_WARPED_ANCHORS, f);
            // A plain path now: a live shape would redraw its old outline.
            *live = None;
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                warp_node(Arc::make_mut(c), w, piece, depth + 1);
            }
        }
        _ => {
            if let Some(b) = n.geometric_bounds() {
                let eps = (b.width().max(b.height()) * 0.05).max(1e-3);
                let a = affine_near(&f, b.center(), eps);
                if a.as_coeffs().iter().all(|x| x.is_finite()) {
                    n.transform(a, Scaling::default());
                }
            }
        }
    }
}
