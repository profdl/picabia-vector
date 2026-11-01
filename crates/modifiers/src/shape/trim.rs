//! Trim: keep a window of a subpath, by fractions of its length.
//!
//! Ported from tlprocedural's Trim Path (`extractSubPath`): on a closed subpath start and end are
//! shifted by the offset and wrapped into `0..1` (the offset slides the window round it, and the
//! window may run through its start point); start = end keeps the whole subpath. Unlike
//! tlprocedural's: an open subpath's start and end are clamped, not wrapped (see [`window`]), and
//! the window is cut from the true curve (curves stay curves), not from a flattened outline.

use vectorcraft_geom::SubPath;

use super::pieces::{Measured, from_pieces, pieces};
use crate::model::Trim;

/// Fractions this close are the same.
const SAME: f64 = 1e-9;

/// What trimming does to one subpath.
#[derive(Debug, PartialEq)]
pub(super) enum Trimmed {
    /// Left as it is (start = end, or nothing to measure).
    Whole,
    /// The window, open.
    Part(SubPath),
    /// The window is empty: the subpath goes.
    Gone,
}

/// `sp` trimmed by `p` (clamped).
pub(super) fn trim(sp: &SubPath, p: &Trim) -> Trimmed {
    if sp.anchors.len() < 2 {
        return Trimmed::Whole;
    }
    let m = Measured::new(pieces(sp));
    if m.total <= SAME {
        return Trimmed::Whole;
    }
    let Some((s, e)) = window(p.start, p.end, p.offset, sp.closed) else { return Trimmed::Whole };
    if !sp.closed && e - s < SAME {
        // The offset slid an open path's window off its end.
        return Trimmed::Gone;
    }
    let mut cut = if s < e {
        m.cut(s * m.total, e * m.total)
    } else {
        // Through the start point: to the end, then from the start.
        let mut a = m.cut(s * m.total, m.total);
        a.extend(m.cut(0.0, e * m.total));
        a
    };
    cut.retain(|p| p.c.p0.is_finite() && p.c.p3.is_finite());
    if cut.is_empty() { Trimmed::Gone } else { Trimmed::Part(from_pieces(&cut, false)) }
}

/// The window's start and end in `0..1`; `None` when start = end (the whole subpath).
///
/// Closed: both shifted by `offset` and wrapped into `0..1` (`start > end`: through the start
/// point). Open: both shifted and **clamped** to `0..1` (end = 1 stays the end), then put in
/// order; the offset can slide the window off the end (start = end after clamping: nothing left).
/// tlprocedural wrapped open paths too, so `end: 1` read as 0 and kept `0..start`: fixed here.
pub(super) fn window(start: f64, end: f64, offset: f64, closed: bool) -> Option<(f64, f64)> {
    if closed {
        let wrap = |x: f64| {
            let w = x.rem_euclid(1.0);
            // `rem_euclid` can round up to exactly 1.
            if w >= 1.0 { 0.0 } else { w }
        };
        let (s, e) = (wrap(start + offset), wrap(end + offset));
        return ((s - e).abs() >= SAME).then_some((s, e));
    }
    if (start - end).abs() < SAME {
        return None;
    }
    let clamp = |x: f64| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
    let (s, e) = (clamp(start + offset), clamp(end + offset));
    Some(if s > e { (e, s) } else { (s, e) })
}
