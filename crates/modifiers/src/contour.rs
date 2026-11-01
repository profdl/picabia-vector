//! Contouring a sampled scalar field into closed rings (marching squares), for Screen.
//!
//! Written for Picabia from the textbook algorithm. It is not derived from the `marching-squares`
//! npm package (AGPL) that tlprocedural uses, nor from any other GPL/AGPL code.
//!
//! # The field
//! A [`Grid`] holds `cols × rows` samples, row-major, `values[j * cols + i]` at
//! `origin + (i·step, j·step)` (y grows downward, as in the document). A sample is **inside** when it
//! is finite and `>= level`; NaN and ±∞ are outside.
//!
//! # The method
//! - The grid is framed by a ring of virtual outside samples, so every region closes: a region that
//!   reaches the grid's edge is closed along that edge (through the edge samples).
//! - Each cell's four corners give one of 16 cases ([`CASES`]); each case lists its directed
//!   segments between edge crossings. The two saddles (5, 10) are decided by the average of the four
//!   corners (the cell centre): centre inside joins the two inside corners, centre outside keeps them
//!   apart.
//! - A crossing is placed on its edge by linear interpolation between the two samples. A crossing
//!   next to a non-finite or virtual sample sits on the finite sample.
//! - Every segment runs with the inside on its right (on screen, y down). A crossing is shared by
//!   the two cells either side of its edge, one leaving it, one arriving, so the segments link into
//!   closed rings with nothing left over. Outer boundaries come out clockwise on screen (positive
//!   [`vectorcraft_geom::SubPath::signed_area`]) and holes counter-clockwise, so even-odd and
//!   non-zero fill draw the same thing.
//! - Repeated points and points on a straight line between their neighbours are dropped; rings that
//!   enclose no area are dropped.
//!
//! # Caps
//! At most [`MAX_CELLS`] cells (a 1000 × 1000-cell grid fits); a bigger grid, a size under 2 × 2,
//! a non-finite or non-positive step, a non-finite origin or level, or a value count that doesn't
//! match the size is refused with a [`ContourError`]. Nothing here panics.

use std::fmt;

use vectorcraft_geom::{FillRule, PathData, Point, SubPath};

/// The most cells a grid may have (`(cols - 1) × (rows - 1)`).
pub const MAX_CELLS: usize = 1_000_000;

/// Why a grid or a contour was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ContourError {
    /// Fewer than 2 samples across or down.
    TooSmall { cols: usize, rows: usize },
    /// More than [`MAX_CELLS`] cells.
    TooLarge { cells: usize },
    /// The values don't match `cols × rows`.
    WrongLength { expected: usize, got: usize },
    /// The step isn't finite and positive.
    BadStep,
    /// The origin isn't finite.
    BadOrigin,
    /// The level (or a band edge) isn't finite, or a band's low edge is above its high edge.
    BadLevel,
}

impl fmt::Display for ContourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContourError::TooSmall { cols, rows } => write!(f, "a contour grid needs at least 2 × 2 samples (got {cols} × {rows})"),
            ContourError::TooLarge { cells } => write!(f, "a contour grid holds at most {MAX_CELLS} cells (asked for {cells})"),
            ContourError::WrongLength { expected, got } => write!(f, "a contour grid needs {expected} values (got {got})"),
            ContourError::BadStep => f.write_str("a contour grid's step must be a positive number"),
            ContourError::BadOrigin => f.write_str("a contour grid's origin must be finite"),
            ContourError::BadLevel => f.write_str("a contour level must be finite (and a band's low edge at most its high edge)"),
        }
    }
}

impl std::error::Error for ContourError {}

/// A sampled scalar field (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    cols: usize,
    rows: usize,
    origin: Point,
    step: f64,
    values: Vec<f64>,
}

/// `cols × rows` checked against the caps; the sample count.
fn check_size(cols: usize, rows: usize, origin: Point, step: f64) -> Result<usize, ContourError> {
    if cols < 2 || rows < 2 {
        return Err(ContourError::TooSmall { cols, rows });
    }
    let cells = (cols - 1).saturating_mul(rows - 1);
    if cells > MAX_CELLS {
        return Err(ContourError::TooLarge { cells });
    }
    if !(step.is_finite() && step > 0.0) {
        return Err(ContourError::BadStep);
    }
    if !(origin.x.is_finite() && origin.y.is_finite()) {
        return Err(ContourError::BadOrigin);
    }
    // cells <= MAX_CELLS with cols, rows >= 2 bounds this well below overflow.
    Ok(cols * rows)
}

impl Grid {
    /// A grid from its samples, row-major (`values[j * cols + i]`).
    pub fn new(cols: usize, rows: usize, origin: Point, step: f64, values: Vec<f64>) -> Result<Grid, ContourError> {
        let expected = check_size(cols, rows, origin, step)?;
        if values.len() != expected {
            return Err(ContourError::WrongLength { expected, got: values.len() });
        }
        Ok(Grid { cols, rows, origin, step, values })
    }

    /// A grid sampling `f` at every sample position.
    pub fn sample(cols: usize, rows: usize, origin: Point, step: f64, mut f: impl FnMut(Point) -> f64) -> Result<Grid, ContourError> {
        let n = check_size(cols, rows, origin, step)?;
        let mut values = Vec::with_capacity(n);
        for j in 0..rows {
            for i in 0..cols {
                values.push(f(Point::new(origin.x + i as f64 * step, origin.y + j as f64 * step)));
            }
        }
        Ok(Grid { cols, rows, origin, step, values })
    }

    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn origin(&self) -> Point {
        self.origin
    }
    pub fn step(&self) -> f64 {
        self.step
    }
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// The sample at column `i`, row `j`; `None` off the grid.
    pub fn value(&self, i: usize, j: usize) -> Option<f64> {
        if i >= self.cols || j >= self.rows {
            return None;
        }
        self.values.get(j * self.cols + i).copied()
    }

    /// The position of sample `(i, j)`.
    fn position(&self, i: usize, j: usize) -> Point {
        Point::new(self.origin.x + i as f64 * self.step, self.origin.y + j as f64 * self.step)
    }
}

/// One closed boundary of a contoured region. The first point isn't repeated at the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    pub points: Vec<Point>,
}

impl Ring {
    /// Twice the signed area by the shoelace formula: positive clockwise on screen (y down), as
    /// [`vectorcraft_geom::SubPath::signed_area`] counts it, halved.
    pub fn signed_area(&self) -> f64 {
        let n = self.points.len();
        let mut a = 0.0;
        for (k, p) in self.points.iter().enumerate() {
            let Some(q) = self.points.get((k + 1) % n.max(1)) else { continue };
            a += p.x * q.y - q.x * p.y;
        }
        a * 0.5
    }

    /// A hole: counter-clockwise on screen.
    pub fn is_hole(&self) -> bool {
        self.signed_area() < 0.0
    }

    /// The ring as a closed polyline subpath.
    pub fn to_subpath(&self) -> SubPath {
        SubPath::polyline(&self.points, true)
    }
}

/// Rings as one compound path's subpaths (non-zero fill, which their orientation makes agree with
/// even-odd).
pub fn rings_to_path(rings: &[Ring]) -> (PathData, FillRule) {
    (PathData::new(rings.iter().map(Ring::to_subpath).collect()), FillRule::NonZero)
}

/// The cell edges, as the [`CASES`] table names them: top, right, bottom, left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    T,
    R,
    B,
    L,
}

/// One case's segments, from one edge's crossing to another's, the inside on the right.
pub type CaseSegments = &'static [(Edge, Edge)];

/// The 16 cases, indexed by the inside corners: top-left 8, top-right 4, bottom-right 2,
/// bottom-left 1. Each entry is `(segments, saddle segments when the centre is inside)`; only
/// cases 5 and 10 have the second.
pub const CASES: [(CaseSegments, CaseSegments); 16] = {
    use Edge::{B, L, R, T};
    [
        (&[], &[]),                             // 0: all outside
        (&[(L, B)], &[]),                       // 1: bottom-left
        (&[(B, R)], &[]),                       // 2: bottom-right
        (&[(L, R)], &[]),                       // 3: bottom half
        (&[(R, T)], &[]),                       // 4: top-right
        (&[(L, B), (R, T)], &[(L, T), (R, B)]), // 5: saddle, top-right + bottom-left
        (&[(B, T)], &[]),                       // 6: right half
        (&[(L, T)], &[]),                       // 7: all but top-left
        (&[(T, L)], &[]),                       // 8: top-left
        (&[(T, B)], &[]),                       // 9: left half
        (&[(T, L), (B, R)], &[(T, R), (B, L)]), // 10: saddle, top-left + bottom-right
        (&[(T, R)], &[]),                       // 11: all but top-right
        (&[(R, L)], &[]),                       // 12: top half
        (&[(R, B)], &[]),                       // 13: all but bottom-right
        (&[(B, L)], &[]),                       // 14: all but bottom-left
        (&[], &[]),                             // 15: all inside
    ]
};

/// Marks a free slot in the link table.
const NONE: u32 = u32::MAX;

/// The rings bounding where `field >= level` (see the module docs).
pub fn contours(field: &Grid, level: f64) -> Result<Vec<Ring>, ContourError> {
    if !level.is_finite() {
        return Err(ContourError::BadLevel);
    }
    Ok(March::new(field, |v| v.is_finite() && v >= level, |v| v - level).run())
}

/// The rings bounding where `lo <= field <= hi`.
///
/// Contours `min(field - lo, hi - field)` at 0, which is exact at every sample and linear between
/// samples except where the band's two edges cross the same cell edge; a band narrower than a cell
/// that lies wholly between two samples is missed.
pub fn bands(field: &Grid, lo: f64, hi: f64) -> Result<Vec<Ring>, ContourError> {
    if !(lo.is_finite() && hi.is_finite() && lo <= hi) {
        return Err(ContourError::BadLevel);
    }
    let g = |v: f64| (v - lo).min(hi - v);
    Ok(March::new(field, |v| v.is_finite() && g(v) >= 0.0, g).run())
}

/// One contouring pass. The padded grid has a virtual outside sample all round: padded sample
/// `(pi, pj)` is grid sample `(pi - 1, pj - 1)`.
struct March<'a, I, S> {
    field: &'a Grid,
    inside: I,
    /// The value interpolated along an edge (crosses 0 at the boundary).
    signed: S,
    /// Padded width and height (samples).
    pw: usize,
    ph: usize,
}

impl<'a, I: Fn(f64) -> bool, S: Fn(f64) -> f64> March<'a, I, S> {
    fn new(field: &'a Grid, inside: I, signed: S) -> Self {
        March { field, inside, signed, pw: field.cols + 2, ph: field.rows + 2 }
    }

    /// The real value at padded sample `(pi, pj)`; `None` for the virtual frame.
    fn at(&self, pi: usize, pj: usize) -> Option<f64> {
        if pi == 0 || pj == 0 {
            return None;
        }
        self.field.value(pi - 1, pj - 1)
    }

    fn is_in(&self, pi: usize, pj: usize) -> bool {
        self.at(pi, pj).is_some_and(|v| (self.inside)(v))
    }

    /// The key of the horizontal edge from padded `(pi, pj)` to `(pi + 1, pj)`.
    fn h(&self, pi: usize, pj: usize) -> usize {
        (pj * self.pw + pi) * 2
    }
    /// The key of the vertical edge from padded `(pi, pj)` to `(pi, pj + 1)`.
    fn v(&self, pi: usize, pj: usize) -> usize {
        (pj * self.pw + pi) * 2 + 1
    }

    /// Where the boundary crosses the edge `key`.
    fn crossing(&self, key: usize) -> Point {
        let cell = key / 2;
        let (pi, pj) = (cell % self.pw, cell / self.pw);
        let (qi, qj) = if key.is_multiple_of(2) { (pi + 1, pj) } else { (pi, pj + 1) };
        let a = self.sample_pos(pi, pj);
        let b = self.sample_pos(qi, qj);
        let (va, vb) = (self.at(pi, pj), self.at(qi, qj));
        let t = match (va.filter(|v| v.is_finite()), vb.filter(|v| v.is_finite())) {
            (Some(va), Some(vb)) => {
                let (sa, sb) = ((self.signed)(va), (self.signed)(vb));
                let d = sa - sb;
                if d.is_finite() && d != 0.0 { (sa / d).clamp(0.0, 1.0) } else { 0.5 }
            }
            // The crossing sits on the finite sample.
            (Some(_), None) => 0.0,
            (None, Some(_)) => 1.0,
            (None, None) => 0.5,
        };
        Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    }

    /// The position of padded sample `(pi, pj)` (the frame lies one step outside the grid).
    fn sample_pos(&self, pi: usize, pj: usize) -> Point {
        if pi >= 1 && pj >= 1 && pi - 1 < self.field.cols && pj - 1 < self.field.rows {
            return self.field.position(pi - 1, pj - 1);
        }
        let s = self.field.step;
        Point::new(self.field.origin.x + (pi as f64 - 1.0) * s, self.field.origin.y + (pj as f64 - 1.0) * s)
    }

    fn run(&self) -> Vec<Ring> {
        let keys = self.pw * self.ph * 2;
        let mut next = vec![NONE; keys];
        let mut any = false;
        for pj in 0..self.ph - 1 {
            for pi in 0..self.pw - 1 {
                let tl = self.is_in(pi, pj);
                let tr = self.is_in(pi + 1, pj);
                let br = self.is_in(pi + 1, pj + 1);
                let bl = self.is_in(pi, pj + 1);
                let case = (usize::from(tl) << 3) | (usize::from(tr) << 2) | (usize::from(br) << 1) | usize::from(bl);
                let Some(&(plain, joined)) = CASES.get(case) else { continue };
                if plain.is_empty() {
                    continue;
                }
                let segments = if !joined.is_empty() && self.centre_inside(pi, pj) { joined } else { plain };
                for &(from, to) in segments {
                    let (a, b) = (self.edge_key(pi, pj, from), self.edge_key(pi, pj, to));
                    // Each crossing leaves exactly once; a clash would mean a broken table.
                    // Keys number twice the padded samples (a few million at most), far below u32::MAX.
                    let Ok(b) = u32::try_from(b) else { continue };
                    if let Some(slot) = next.get_mut(a)
                        && *slot == NONE
                    {
                        *slot = b;
                        any = true;
                    }
                }
            }
        }
        if !any {
            return Vec::new();
        }
        let mut rings = Vec::new();
        for start in 0..keys {
            if next.get(start).is_none_or(|&n| n == NONE) {
                continue;
            }
            let mut points = Vec::new();
            let mut key = start;
            // A ring visits each crossing once, so it can't be longer than the table.
            for _ in 0..keys {
                let Some(slot) = next.get_mut(key) else { break };
                let n = *slot;
                if n == NONE {
                    break;
                }
                *slot = NONE;
                points.push(self.crossing(key));
                key = n as usize;
                if key == start {
                    break;
                }
            }
            if let Some(ring) = tidy(points) {
                rings.push(ring);
            }
        }
        rings
    }

    /// The saddle test: the average of the cell's four corners, inside or not. A non-finite corner
    /// (or the frame) keeps the corners apart.
    fn centre_inside(&self, pi: usize, pj: usize) -> bool {
        let corners = [self.at(pi, pj), self.at(pi + 1, pj), self.at(pi + 1, pj + 1), self.at(pi, pj + 1)];
        let mut sum = 0.0;
        for c in corners {
            match c {
                Some(v) if v.is_finite() => sum += (self.signed)(v),
                _ => return false,
            }
        }
        let mean = sum * 0.25;
        mean.is_finite() && mean >= 0.0
    }

    fn edge_key(&self, pi: usize, pj: usize, e: Edge) -> usize {
        match e {
            Edge::T => self.h(pi, pj),
            Edge::B => self.h(pi, pj + 1),
            Edge::L => self.v(pi, pj),
            Edge::R => self.v(pi + 1, pj),
        }
    }
}

/// Drops repeated points and points on a straight line between their neighbours; `None` for a
/// ring that encloses no area.
fn tidy(mut points: Vec<Point>) -> Option<Ring> {
    points.dedup();
    while points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    // Remove collinear points until none is left (each pass is O(n); points only go).
    loop {
        let n = points.len();
        if n < 3 {
            return None;
        }
        let mut kept = Vec::with_capacity(n);
        for k in 0..n {
            let (Some(&p), Some(&q), Some(&r)) = (points.get((k + n - 1) % n), points.get(k), points.get((k + 1) % n)) else {
                continue;
            };
            let cross = (q.x - p.x) * (r.y - q.y) - (q.y - p.y) * (r.x - q.x);
            let scale = ((q - p).hypot() * (r - q).hypot()).max(f64::MIN_POSITIVE);
            if q == p || (cross / scale).abs() <= 1e-12 {
                continue;
            }
            kept.push(q);
        }
        if kept.len() == n {
            break;
        }
        points = kept;
    }
    let ring = Ring { points };
    (ring.signed_area().abs() > 0.0).then_some(ring)
}
