//! Trim: keep a window of a subpath, by fractions of its length.
//!
//! Ported from tlprocedural's Trim Path (`extractSubPath`): start and end are shifted by the
//! offset and wrapped into `0..1` (never clamped, so the offset slides the window round a closed
//! path); a closed subpath's window may run through its start point, an open one's is put in order
//! instead; start = end keeps the whole subpath. Unlike tlprocedural's, the window is cut from the
//! true curve (curves stay curves), not from a flattened outline.

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
    let Some((s, e)) = window(p.start + p.offset, p.end + p.offset, sp.closed) else { return Trimmed::Whole };
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

/// The window's start and end in `0..1` (`start > end`: through the start point, closed only);
/// `None` when they are the same (the whole subpath).
pub(super) fn window(start: f64, end: f64, closed: bool) -> Option<(f64, f64)> {
    let wrap = |x: f64| {
        let w = x.rem_euclid(1.0);
        // `rem_euclid` can round up to exactly 1.
        if w >= 1.0 { 0.0 } else { w }
    };
    let (mut s, mut e) = (wrap(start), wrap(end));
    if !closed && s > e {
        std::mem::swap(&mut s, &mut e);
    }
    ((s - e).abs() >= SAME).then_some((s, e))
}
