//! The path operations the stencil runs on (Picabia): raw `linesweeper` booleans on kurbo paths,
//! plus the small path helpers the pipeline shares. Region offset, opening and closing are in
//! [`crate::offset`].
//!
//! Why not `vectorcraft_pathops::try_boolean`: VectorCraft's booleans work on `PathData` and tidy
//! their result for editing (split curve pieces refitted, contours thinner than its 0.01-point
//! precision dropped). The stencil runs dozens of booleans per sheet on flattened polygons and
//! relies on the sweep's raw contours (plain, non-overlapping, outer contours wound positively,
//! holes negatively, nothing dropped) for the islands, the paper and the checks. Both use the same
//! sweep (`linesweeper`, the workspace version), so the geometry agrees.
//!
//! Results are even-odd plain contours; non-zero reads them the same.

use vectorcraft_geom::kurbo::{self, BezPath, PathEl, Point};

/// The corner shape of an offset: kurbo's stroke joins (`Bevel`, `Miter`, `Round`).
pub use kurbo::Join as StrokeJoin;

/// A boolean operation between two filled regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoolOp {
    /// A ∪ B.
    Union,
    /// A − B.
    Subtract,
    /// A ∩ B.
    Intersect,
    /// A ⊕ B (where exactly one of them is).
    Exclude,
}

/// Why a path operation made nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathOpsError {
    /// The inputs contain NaN, infinite or huge coordinates, are too large, or the sweep failed
    /// on them.
    BadInput,
}

impl std::fmt::Display for PathOpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "path contains invalid coordinates")
    }
}

impl std::error::Error for PathOpsError {}

/// Close every open subpath (a filled open path paints as if closed).
pub fn closed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(*el);
    }
    if open {
        out.close_path();
    }
    out
}

/// Every coordinate is finite and under 1e9 in magnitude (the sweep needs bounded input).
pub fn finite(path: &BezPath) -> bool {
    let ok = |p: &Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9;
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => ok(p),
        PathEl::QuadTo(a, b) => ok(a) && ok(b),
        PathEl::CurveTo(a, b, c) => ok(a) && ok(b) && ok(c),
        PathEl::ClosePath => true,
    })
}

/// Each subpath of `path` as a path of its own, in order.
pub fn split_subpaths(path: &BezPath) -> Vec<BezPath> {
    let mut out = Vec::new();
    let mut cur = BezPath::new();
    for el in path.elements() {
        if let PathEl::MoveTo(_) = el
            && !cur.elements().is_empty()
        {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(*el);
    }
    if !cur.elements().is_empty() {
        out.push(cur);
    }
    out
}

/// Run the sweep on `f`. `linesweeper` has panicked on rare near-degenerate input (an internal
/// `unwrap` while ordering curves): that is bad input, not a crash. VectorCraft's own booleans
/// guard their sweep the same way.
// shortcut: no catch where panics abort (wasm32); drop this once linesweeper is known not to panic there.
fn sweep<T>(f: impl FnOnce() -> Result<T, PathOpsError>) -> Result<T, PathOpsError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(PathOpsError::BadInput))
}

/// `a op b`, both read non-zero.
pub(crate) fn binary(a: &BezPath, b: &BezPath, op: linesweeper::BinaryOp) -> Result<BezPath, PathOpsError> {
    binary_by(a, b, linesweeper::FillRule::NonZero, op)
}

/// [`binary`] with both operands read by fill rule `rule`.
fn binary_by(a: &BezPath, b: &BezPath, rule: linesweeper::FillRule, op: linesweeper::BinaryOp) -> Result<BezPath, PathOpsError> {
    if a.elements().is_empty() && b.elements().is_empty() {
        return Ok(BezPath::new());
    }
    if !finite(a) || !finite(b) {
        return Err(PathOpsError::BadInput);
    }
    let contours = sweep(|| linesweeper::binary_op(&closed(a), &closed(b), rule, op).map_err(|_| PathOpsError::BadInput))?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.elements().iter().copied());
    }
    Ok(out)
}

/// Normalise a single path's winding (non-zero) into plain even-odd contours.
pub fn simplify(path: &BezPath) -> Result<BezPath, PathOpsError> {
    binary(path, &BezPath::new(), linesweeper::BinaryOp::Union)
}

/// Normalise one path read by its fill rule (`even_odd`, else non-zero) into plain contours.
pub fn simplify_with_rule(path: &BezPath, even_odd: bool) -> Result<BezPath, PathOpsError> {
    let rule = if even_odd { linesweeper::FillRule::EvenOdd } else { linesweeper::FillRule::NonZero };
    binary_by(path, &BezPath::new(), rule, linesweeper::BinaryOp::Union)
}

/// `acc op p`, both read non-zero (open subpaths closed).
pub fn boolean_step(acc: &BezPath, p: &BezPath, op: BoolOp) -> Result<BezPath, PathOpsError> {
    let op = match op {
        BoolOp::Union => linesweeper::BinaryOp::Union,
        BoolOp::Intersect => linesweeper::BinaryOp::Intersection,
        BoolOp::Exclude => linesweeper::BinaryOp::Xor,
        BoolOp::Subtract => linesweeper::BinaryOp::Difference,
    };
    binary(acc, p, op)
}
