//! Repeat-aware islands and ties: when the design is a translation repeat (a wallpaper or grid row
//! above the stencil), judge the paper as it prints, across the cell edges.
//!
//! The lattice cell `C` nearest the design's centre is picked, and the periodic design is built on
//! the 3×3-cell block `B` around it: every paint translated by the lattice vectors that make it
//! meet `B`, exact duplicates dropped (same colour, same geometry within a millionth of the cell;
//! the lowest paint order kept). Visible regions, selection and clean-up run on that, clipped to
//! `B`. The islands are the paper pieces of `B` that don't reach its outline (bounded in the
//! infinite repeat), and those whose bounds centre lies in `C` are the cell's islands
//! ([`crate::Stats::repeat_islands`]). Every island of `B` is tied (so a boxed-in one can reach a
//! neighbour), and the bars of the cell's islands are kept: repeated over every lattice translate
//! that meets the sheet, they hold every copy the same way. A repeat with one zero vector (a grid
//! of one row or column) is a strip: its cell spans the design across the repeat, and the block is
//! 1×3 cells, grown by the margin across (the sheet's paper holds there).

use std::collections::HashMap;

use vectorcraft_geom::kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Vec2};
use vectorcraft_modifiers::Repeat;

use crate::Paint;
use crate::ties::Bar;

/// The most lattice cells the sheet may span; a finer repeat is ignored with a warning.
pub(crate) const MAX_CELLS: usize = 10_000;
/// The most translated paints tried for the block (before duplicates are dropped).
pub(crate) const MAX_BLOCK_CANDIDATES: usize = 200_000;
/// The most tie bars repeated over the sheet.
pub(crate) const MAX_REPEAT_BARS: usize = 100_000;

/// The repeat cell a tile is cut from: the cell of `r`'s lattice containing the centre of
/// `design` (with `margin` around it), its corners `o, o + a, o + a + b, o + b`; a strip's cell
/// spans the design across the repeat. `None` where the stencil ignores the repeat (degenerate, or
/// over [`MAX_CELLS`] cells on the sheet). The cell [`crate::compute_repeat`] tiles: frames and
/// bounds use it before a result exists.
pub fn tile_cell(r: Repeat, design: Rect, margin: f64) -> Option<[Point; 4]> {
    let lat = Lattice::new(r, design, margin).ok()?;
    let o = lat.cell_origin(lat.cell_at(design.abs().center()));
    let c = [o, o + lat.a, o + lat.a + lat.b, o + lat.b];
    c.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(c)
}

fn cross(u: Vec2, v: Vec2) -> f64 {
    u.x * v.y - u.y * v.x
}

/// A translation lattice in the layer's space, ready to use: `b` is never zero (a strip's is the
/// design's extent across `a`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lattice {
    pub origin: Point,
    pub a: Vec2,
    pub b: Vec2,
    /// Repeats along `b` too (else a strip).
    pub two_d: bool,
    /// A strip's block reaches this far past the design across `a` (the margin).
    pub pad: f64,
}

/// A lattice cell by its numbers along `a` and `b`.
pub(crate) type Cell = (i64, i64);

impl Lattice {
    /// The lattice of `r` for a design in `design` with a `margin`, or why it can't be used.
    pub fn new(r: Repeat, design: Rect, margin: f64) -> Result<Lattice, String> {
        let degenerate = || "the repeat's lattice is degenerate (its vectors are zero or parallel); the stencil ignores the repeat".to_string();
        let finite = [r.origin.x, r.origin.y, r.a.x, r.a.y, r.b.x, r.b.y].iter().all(|v| v.is_finite());
        let size = design.width().abs().max(design.height().abs());
        if !(finite && size.is_finite() && size > 0.0) {
            return Err(degenerate());
        }
        let eps = size * 1e-9;
        let (la, lb) = (r.a.hypot(), r.b.hypot());
        let lat = match (la > eps, lb > eps) {
            (false, false) => return Err(degenerate()),
            (true, true) => {
                if cross(r.a, r.b).abs() <= la * lb * 1e-9 {
                    return Err(degenerate());
                }
                Lattice { origin: r.origin, a: r.a, b: r.b, two_d: true, pad: 0.0 }
            }
            (one_a, _) => {
                let (a, len) = if one_a { (r.a, la) } else { (r.b, lb) };
                let n = Vec2::new(-a.y, a.x) / len;
                let d = design.abs();
                let across = [d.origin(), Point::new(d.x1, d.y0), Point::new(d.x1, d.y1), Point::new(d.x0, d.y1)].map(|p| (p - r.origin).dot(n));
                let (u0, u1) = across.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &u| (lo.min(u), hi.max(u)));
                if u1 - u0 <= eps {
                    return Err(degenerate());
                }
                Lattice { origin: r.origin + n * u0, a, b: n * (u1 - u0), two_d: false, pad: margin.max(0.0) }
            }
        };
        let sheet = design.abs().inflate(margin, margin);
        let (s0, s1, t0, t1) = lat.range(sheet);
        let cells = (s1.ceil() - s0.floor()) * if lat.two_d { t1.ceil() - t0.floor() } else { 1.0 };
        if !(cells.is_finite() && cells <= MAX_CELLS as f64) {
            return Err(format!("the repeat is too fine for the sheet (over {MAX_CELLS} cells); the stencil ignores the repeat"));
        }
        Ok(lat)
    }

    /// `p` in lattice coordinates: `p = origin + s·a + t·b`.
    pub fn coords(&self, p: Point) -> (f64, f64) {
        let d = p - self.origin;
        let det = cross(self.a, self.b);
        (cross(d, self.b) / det, cross(self.a, d) / det)
    }

    /// The lattice coordinates `(s0, s1, t0, t1)` spanned by `r`'s corners.
    pub fn range(&self, r: Rect) -> (f64, f64, f64, f64) {
        let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        corners.iter().map(|&p| self.coords(p)).fold((f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY), |(a, b, c, d), (s, t)| {
            (a.min(s), b.max(s), c.min(t), d.max(t))
        })
    }

    /// The cell containing `p` (a strip's has number 0 across).
    pub fn cell_at(&self, p: Point) -> Cell {
        let (s, t) = self.coords(p);
        (s.floor() as i64, if self.two_d { t.floor() as i64 } else { 0 })
    }

    pub fn shift(&self, k: i64, l: i64) -> Vec2 {
        self.a * k as f64 + self.b * l as f64
    }

    /// Cell `c`'s corner at its lowest `s` and `t`.
    pub fn cell_origin(&self, c: Cell) -> Point {
        self.origin + self.shift(c.0, c.1)
    }

    pub fn in_cell(&self, c: Cell, p: Point) -> bool {
        let (s, t) = self.coords(p);
        let si = s - c.0 as f64;
        let ti = t - c.1 as f64;
        (0.0..1.0).contains(&si) && (!self.two_d || (0.0..1.0).contains(&ti))
    }

    /// The parallelogram at `o` spanned by `u` and `v`, wound positively.
    fn parallelogram(o: Point, u: Vec2, v: Vec2) -> BezPath {
        let mut ring = vec![o, o + u, o + u + v, o + v];
        if crate::islands::ring_area(&ring) < 0.0 {
            ring.reverse();
        }
        crate::rings::to_path(&[ring])
    }

    /// Cell `c`'s outline.
    pub fn cell_path(&self, c: Cell) -> BezPath {
        Self::parallelogram(self.cell_origin(c), self.a, self.b)
    }

    /// The block around cell `c`: 3×3 cells, or a strip's 1×3 grown by `pad` across.
    pub fn block_path(&self, c: Cell) -> BezPath {
        let o = self.cell_origin(c);
        if self.two_d {
            Self::parallelogram(o - self.a - self.b, self.a * 3.0, self.b * 3.0)
        } else {
            let n = self.b / self.b.hypot();
            Self::parallelogram(o - self.a - n * self.pad, self.a * 3.0, self.b + n * (2.0 * self.pad))
        }
    }

    /// The lattice coordinates the block around `c` spans.
    fn block_range(&self, c: Cell) -> (f64, f64, f64, f64) {
        let (i, j) = (c.0 as f64, c.1 as f64);
        if self.two_d {
            (i - 1.0, i + 2.0, j - 1.0, j + 2.0)
        } else {
            let p = self.pad / self.b.hypot();
            (i - 1.0, i + 2.0, -p, 1.0 + p)
        }
    }

    /// The shifts `(k, l)` that move something spanning `from` (lattice coordinates) to meet
    /// `to`, along `b` only for a 2D lattice.
    pub fn shifts(&self, from: (f64, f64, f64, f64), to: (f64, f64, f64, f64)) -> Vec<(i64, i64)> {
        let span = |lo: f64, hi: f64, tlo: f64, thi: f64| -> Option<(i64, i64)> {
            let (k0, k1) = ((tlo - hi).ceil(), (thi - lo).floor());
            (k0.is_finite() && k1.is_finite() && k0 <= k1 && k1 - k0 <= MAX_CELLS as f64).then_some((k0 as i64, k1 as i64))
        };
        let Some((k0, k1)) = span(from.0, from.1, to.0, to.1) else { return Vec::new() };
        let (l0, l1) = if self.two_d {
            match span(from.2, from.3, to.2, to.3) {
                Some(r) => r,
                None => return Vec::new(),
            }
        } else if from.3 >= to.2 && from.2 <= to.3 {
            (0, 0)
        } else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for k in k0..=k1 {
            for l in l0..=l1 {
                out.push((k, l));
                if out.len() > MAX_CELLS {
                    return out;
                }
            }
        }
        out
    }
}

/// `a` and `b` are the same outline within `tol`.
fn same(a: &BezPath, b: &BezPath, tol: f64) -> bool {
    let close = |p: Point, q: Point| (p - q).hypot() <= tol;
    a.elements().len() == b.elements().len()
        && a.elements().iter().zip(b.elements()).all(|(x, y)| match (x, y) {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => close(*p, *q),
            (PathEl::QuadTo(p1, p2), PathEl::QuadTo(q1, q2)) => close(*p1, *q1) && close(*p2, *q2),
            (PathEl::CurveTo(p1, p2, p3), PathEl::CurveTo(q1, q2, q3)) => close(*p1, *q1) && close(*p2, *q2) && close(*p3, *q3),
            (PathEl::ClosePath, PathEl::ClosePath) => true,
            _ => false,
        })
}

/// The periodic design on the block around cell `c`: every paint translated to meet the block,
/// duplicates dropped (see the module docs), bottom first. `Err` with a warning past
/// [`MAX_BLOCK_CANDIDATES`] or [`crate::MAX_PAINTS`].
pub(crate) fn block_paints(paints: &[Paint], lat: &Lattice, c: Cell) -> Result<Vec<Paint>, String> {
    let to = lat.block_range(c);
    let q = lat.a.hypot().max(lat.b.hypot()) * 1e-6;
    let key = |p: &Paint, bb: Rect, dx: i64, dy: i64| {
        (p.colour.map(f32::to_bits), (bb.x0 / q).round() as i64 + dx, (bb.y0 / q).round() as i64 + dy, p.path.elements().len())
    };
    let mut out: Vec<Paint> = Vec::new();
    let mut seen: HashMap<([u32; 4], i64, i64, usize), Vec<usize>> = HashMap::new();
    let mut tried = 0usize;
    for p in paints {
        let bb = p.path.bounding_box();
        if !bb.is_finite() {
            continue;
        }
        for (k, l) in lat.shifts(lat.range(bb), to) {
            tried += 1;
            if tried > MAX_BLOCK_CANDIDATES {
                return Err(format!(
                    "the repeat has too many shapes to judge across its edges (over {MAX_BLOCK_CANDIDATES}); the stencil ignores the repeat"
                ));
            }
            let path = Affine::translate(lat.shift(k, l)) * p.path.clone();
            let cand = Paint { path, colour: p.colour, order: p.order };
            let cb = cand.path.bounding_box();
            let mut dup = None;
            'look: for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(ids) = seen.get(&key(&cand, cb, dx, dy))
                        && let Some(&i) = ids.iter().find(|&&i| out.get(i).is_some_and(|o| same(&o.path, &cand.path, q * 10.0)))
                    {
                        dup = Some(i);
                        break 'look;
                    }
                }
            }
            match dup.and_then(|i| out.get_mut(i)) {
                Some(o) => o.order = o.order.min(cand.order),
                None => {
                    seen.entry(key(&cand, cb, 0, 0)).or_default().push(out.len());
                    out.push(cand);
                    if out.len() > crate::MAX_PAINTS {
                        return Err(format!("the repeat's block has more than {} shapes; the stencil ignores the repeat", crate::MAX_PAINTS));
                    }
                }
            }
        }
    }
    // Stable: equal orders keep the order they were made in.
    out.sort_by_key(|p| p.order);
    Ok(out)
}

/// `bar` moved by `v`.
pub(crate) fn moved(bar: &Bar, v: Vec2) -> Bar {
    bar.map(|p| p + v)
}

/// A bar's bounds.
pub(crate) fn bar_bounds(bar: &Bar) -> Rect {
    bar.iter().skip(1).fold(Rect::from_points(bar[0], bar[0]), |b, &p| b.union_pt(p))
}
