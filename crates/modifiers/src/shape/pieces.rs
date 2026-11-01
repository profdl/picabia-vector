//! A subpath as a list of segments ([`Piece`]s), measured by arc length, and back.

use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen};
use vectorcraft_geom::{Anchor, AnchorKind, CubicBez, Point, SubPath};

/// How exactly arc lengths are measured (points).
pub(super) const ARC_ACCURACY: f64 = 1e-4;
/// Pieces shorter than this (points) are dropped as empty.
pub(super) const EMPTY: f64 = 1e-9;

/// One segment: a cubic, or a straight line (its control points are then ignored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Piece {
    pub c: CubicBez,
    pub line: bool,
}

impl Piece {
    pub fn line(a: Point, b: Point) -> Piece {
        Piece { c: CubicBez::new(a, a, b, b), line: true }
    }

    pub fn len(&self) -> f64 {
        if self.line { self.c.p0.distance(self.c.p3) } else { self.c.arclen(ARC_ACCURACY) }
    }

    /// The parameter `d` points along the piece (`len` its length).
    pub fn param_at(&self, d: f64, len: f64) -> f64 {
        if len <= EMPTY {
            return 0.0;
        }
        if self.line { (d / len).clamp(0.0, 1.0) } else { self.c.inv_arclen(d.clamp(0.0, len), ARC_ACCURACY).clamp(0.0, 1.0) }
    }

    pub fn eval(&self, t: f64) -> Point {
        if self.line { self.c.p0.lerp(self.c.p3, t) } else { self.c.eval(t) }
    }

    /// The part from `t0` to `t1`.
    pub fn sub(&self, t0: f64, t1: f64) -> Piece {
        if self.line { Piece::line(self.eval(t0), self.eval(t1)) } else { Piece { c: self.c.subsegment(t0..t1), line: false } }
    }

    /// The two halves at `t = 0.5`.
    pub fn halves(&self) -> (Piece, Piece) {
        (self.sub(0.0, 0.5), self.sub(0.5, 1.0))
    }
}

/// `sp`'s segments, in order (a closed subpath's closing one last).
pub(super) fn pieces(sp: &SubPath) -> Vec<Piece> {
    (0..sp.segment_count())
        .map(|i| {
            let c = sp.segment(i);
            if sp.segment_is_line(i) { Piece::line(c.p0, c.p3) } else { Piece { c, line: false } }
        })
        .collect()
}

/// A subpath through `pieces` (each starting where the one before ends). Open: an anchor at each
/// piece's start and one at the last piece's end; closed: one at each piece's start (the last
/// piece ends at the first anchor). An anchor whose handles are opposite each other is smooth.
pub(super) fn from_pieces(pieces: &[Piece], closed: bool) -> SubPath {
    let Some(first) = pieces.first() else { return SubPath::new(Vec::new(), closed) };
    let mut anchors: Vec<Anchor> = Vec::with_capacity(pieces.len() + 1);
    anchors.push(Anchor::corner(first.c.p0));
    for (i, p) in pieces.iter().enumerate() {
        if let Some(a) = anchors.last_mut() {
            a.h_out = if p.line { a.p } else { p.c.p1 };
        }
        let end = Anchor { p: p.c.p3, h_in: if p.line { p.c.p3 } else { p.c.p2 }, h_out: p.c.p3, kind: AnchorKind::Corner };
        if closed && i + 1 == pieces.len() {
            // The closing piece ends at the first anchor: it brings its in handle.
            if let Some(a) = anchors.first_mut() {
                a.h_in = end.h_in - (end.p - a.p);
            }
        } else {
            anchors.push(end);
        }
    }
    for a in &mut anchors {
        a.kind = kind_of(a);
    }
    SubPath::new(anchors, closed)
}

/// Smooth when both handles exist and point opposite ways.
fn kind_of(a: &Anchor) -> AnchorKind {
    if !a.has_in() || !a.has_out() {
        return AnchorKind::Corner;
    }
    let (i, o) = ((a.h_in - a.p).normalize(), (a.h_out - a.p).normalize());
    if (i + o).hypot() < 1e-6 { AnchorKind::Smooth } else { AnchorKind::Corner }
}

/// A subpath measured by arc length.
pub(super) struct Measured {
    pub pieces: Vec<Piece>,
    /// Each piece's length.
    pub lens: Vec<f64>,
    /// Where each piece starts along the subpath.
    pub starts: Vec<f64>,
    pub total: f64,
}

impl Measured {
    pub fn new(pieces: Vec<Piece>) -> Measured {
        let lens: Vec<f64> = pieces.iter().map(Piece::len).map(|l| if l.is_finite() { l } else { 0.0 }).collect();
        let mut starts = Vec::with_capacity(lens.len());
        let mut total = 0.0;
        for l in &lens {
            starts.push(total);
            total += l;
        }
        Measured { pieces, lens, starts, total }
    }

    /// The piece `d` points along (clamped to the subpath), and how far into it.
    fn locate(&self, d: f64) -> Option<(usize, f64)> {
        let d = d.clamp(0.0, self.total);
        // The last piece starting at or before `d` (empty pieces are skipped past).
        let i = self.starts.partition_point(|s| *s <= d).saturating_sub(1);
        let i = (i..self.pieces.len()).find(|&k| self.lens.get(k).is_some_and(|l| *l > EMPTY) || k + 1 == self.pieces.len())?;
        Some((i, d - self.starts.get(i).copied().unwrap_or(0.0)))
    }

    /// The point `d` points along.
    pub fn point_at(&self, d: f64) -> Option<Point> {
        let (i, local) = self.locate(d)?;
        let (p, len) = (self.pieces.get(i)?, self.lens.get(i).copied().unwrap_or(0.0));
        Some(p.eval(p.param_at(local, len)))
    }

    /// The pieces between `d0` and `d1` (`d0 <= d1`), cut exactly; empty ones left out.
    pub fn cut(&self, d0: f64, d1: f64) -> Vec<Piece> {
        let mut out = Vec::new();
        for (i, p) in self.pieces.iter().enumerate() {
            let (s, len) = (self.starts.get(i).copied().unwrap_or(0.0), self.lens.get(i).copied().unwrap_or(0.0));
            let (lo, hi) = (d0.max(s), d1.min(s + len));
            if hi - lo <= EMPTY {
                continue;
            }
            let (t0, t1) = (p.param_at(lo - s, len), p.param_at(hi - s, len));
            if t1 > t0 {
                out.push(if t0 <= 0.0 && t1 >= 1.0 { *p } else { p.sub(t0, t1) });
            }
        }
        out
    }
}
