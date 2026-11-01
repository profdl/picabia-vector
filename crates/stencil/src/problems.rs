//! Stage 8: checks. What still breaks the cutting limits in the final cut, as [`Problem`]s with
//! their bounds (canvas markers) and counts per [`ProblemKind`] (panel warnings).
//!
//! - **Narrow paper** and **narrow cut**: the region minus its opening. The opening shrinks by the
//!   limit's half width less a slack (the clean-up's simplification tolerance: what it may have
//!   moved an outline by) and grows back by the full half width, so a part that was widened to the
//!   limit and then simplified isn't flagged, and the polygonal round joins leave no seams. What is
//!   left over are the parts narrower than the limit (bridges, slivers, pinches, fine spikes); a
//!   piece under a quarter of the radius squared (a right-angle corner leaves 0.21 r²) is a convex
//!   corner, not a problem. Two offsets per region, four in all. In fix mode the clean-up has
//!   just opened and closed the cut, so only what came after it can be narrow: the checks then
//!   look only near the changes ([`Zones`]: tie and lattice bars, registration holes, the sheet's
//!   or tile's edges), clipped to those zones grown by twice the radius (what the clip itself
//!   leaves along its edges is ignored: only pieces reaching into a zone count).
//! - **Small cut** and **small hole**: cut contours (outers) and paper holes in the cut (islands)
//!   enclosing less than `minCutAreaMm2`.
//! - **Sharp corner**: a vertex of a cut contour where the paper between its edges is narrower in
//!   angle than `minCornerAngle`. Edges are judged over a window of `minCutWidthMm` each way along
//!   the outline (a corner simplified into a run of short edges counts as one); the sharpest
//!   vertex of a run is reported and others within the window of it are not.
//! - **Untied**: the islands left floating (their bounds come from the hold step).
//!
//! The list keeps at most [`MAX_PROBLEMS`], taken in turn from each kind; the counts are complete.

use crate::ops::split_subpaths;
use crate::ops::{BoolOp, StrokeJoin};
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape, Vec2};

use crate::rings;

/// The most problems [`crate::StencilOut::problems`] lists (the counts go on past it).
pub const MAX_PROBLEMS: usize = 500;

/// What breaks a cutting limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProblemKind {
    /// Paper narrower than `minBridgeMm` (between two cuts, or a cut and the sheet's edge).
    NarrowPaper,
    /// A cut narrower than `minCutWidthMm`.
    NarrowCut,
    /// A cut piece smaller than `minCutAreaMm2` (clean-up drops these unless limits are flagged,
    /// so in fix mode only registration holes and ties make them).
    SmallCut,
    /// A paper hole in the cut (an island) smaller than `minCutAreaMm2`.
    SmallHole,
    /// A paper corner sharper than `minCornerAngle`.
    SharpCorner,
    /// A paper island no tie or lattice holds.
    Untied,
}

impl ProblemKind {
    /// Every kind, in the order the counts and lists use.
    pub const ALL: [ProblemKind; 6] = [
        ProblemKind::NarrowPaper,
        ProblemKind::NarrowCut,
        ProblemKind::SmallCut,
        ProblemKind::SmallHole,
        ProblemKind::SharpCorner,
        ProblemKind::Untied,
    ];

    /// The camelCase id (`narrowPaper`, `narrowCut`, `smallCut`, `smallHole`, `sharpCorner`,
    /// `untied`), as `modifier.stencilInfo` writes it.
    pub fn id(self) -> &'static str {
        match self {
            ProblemKind::NarrowPaper => "narrowPaper",
            ProblemKind::NarrowCut => "narrowCut",
            ProblemKind::SmallCut => "smallCut",
            ProblemKind::SmallHole => "smallHole",
            ProblemKind::SharpCorner => "sharpCorner",
            ProblemKind::Untied => "untied",
        }
    }

    /// Plain words for one or more of this kind ("narrow paper bridges"), for warnings.
    pub fn plural(self) -> &'static str {
        match self {
            ProblemKind::NarrowPaper => "narrow paper bridges",
            ProblemKind::NarrowCut => "narrow cuts",
            ProblemKind::SmallCut => "small cuts",
            ProblemKind::SmallHole => "small holes",
            ProblemKind::SharpCorner => "sharp corners",
            ProblemKind::Untied => "untied islands",
        }
    }

    fn slot(self) -> usize {
        match self {
            ProblemKind::NarrowPaper => 0,
            ProblemKind::NarrowCut => 1,
            ProblemKind::SmallCut => 2,
            ProblemKind::SmallHole => 3,
            ProblemKind::SharpCorner => 4,
            ProblemKind::Untied => 5,
        }
    }
}

/// One place that breaks a limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Problem {
    pub kind: ProblemKind,
    /// Where it is, layer units.
    pub bounds: Rect,
}

/// How many problems of each kind the stencil has (all of them, past [`MAX_PROBLEMS`] too).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProblemCounts {
    counts: [usize; 6],
}

impl ProblemCounts {
    pub fn get(&self, kind: ProblemKind) -> usize {
        self.counts.get(kind.slot()).copied().unwrap_or(0)
    }

    pub fn total(&self) -> usize {
        self.counts.iter().sum()
    }

    fn add(&mut self, kind: ProblemKind, n: usize) {
        if let Some(c) = self.counts.get_mut(kind.slot()) {
            *c = c.saturating_add(n);
        }
    }
}

/// The limits the checks use, in layer units (widths, not radii).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CheckLimits {
    pub min_bridge: f64,
    pub min_cut_width: f64,
    pub min_area: f64,
    /// Degrees; 0 checks nothing.
    pub min_corner_angle: f64,
    /// How far the clean-up may have moved an outline (its simplification tolerance).
    pub slack: f64,
}

/// Where the narrow checks look: everywhere, or only near these rectangles (layer units).
#[derive(Clone, Debug)]
pub(crate) enum Zones {
    Everywhere,
    Near(Vec<Rect>),
}

/// The most zone rectangles; past it the checks look everywhere.
pub(crate) const MAX_ZONES: usize = 256;

impl Zones {
    /// `Near(rects)`, or `Everywhere` past [`MAX_ZONES`].
    pub fn near(rects: Vec<Rect>) -> Zones {
        if rects.len() > MAX_ZONES { Zones::Everywhere } else { Zones::Near(rects) }
    }

    /// Bands `width` wide just inside `r`'s four edges.
    pub fn frame(r: Rect, width: f64) -> [Rect; 4] {
        let w = width.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
        [
            Rect::new(r.x0, r.y0, r.x1, r.y0 + w),
            Rect::new(r.x0, r.y1 - w, r.x1, r.y1),
            Rect::new(r.x0, r.y0, r.x0 + w, r.y1),
            Rect::new(r.x1 - w, r.y0, r.x1, r.y1),
        ]
    }
}

/// [`narrow`] near `zones` only: the region clipped to the zones grown by `pad`, and the pieces
/// with a point within `r / 2` of a zone.
fn narrow_near(region: &BezPath, r: f64, slack: f64, zones: &Zones, pad: f64) -> Result<Vec<Rect>, crate::ops::PathOpsError> {
    let rects = match zones {
        Zones::Everywhere => return narrow(region, r, slack),
        Zones::Near(z) => z,
    };
    let shrink = r - slack;
    if rects.is_empty() || shrink.is_nan() || shrink <= 0.0 || region.elements().is_empty() {
        return Ok(Vec::new());
    }
    let mut clip = BezPath::new();
    for z in rects {
        clip.extend(z.abs().inflate(pad, pad).to_path(0.1).iter());
    }
    let local = crate::ops::boolean_step(region, &clip, BoolOp::Intersect)?;
    let reach: Vec<Rect> = rects.iter().map(|z| z.abs().inflate(r / 2.0, r / 2.0)).collect();
    let pieces = narrow_pieces(&local, r, slack)?;
    Ok(pieces
        .into_iter()
        .filter(|p| p.elements().iter().filter_map(|e| e.end_point()).any(|v| reach.iter().any(|z| z.contains(v))))
        .map(|p| p.bounding_box())
        .collect())
}

/// The checks' result.
#[derive(Debug, Default)]
pub(crate) struct Checked {
    pub problems: Vec<Problem>,
    pub counts: ProblemCounts,
    pub warnings: Vec<String>,
}

/// Places per kind as found (each list capped at [`MAX_PROBLEMS`]) with the full counts.
#[derive(Default)]
struct Found {
    lists: [Vec<Rect>; 6],
    counts: ProblemCounts,
}

impl Found {
    fn push(&mut self, kind: ProblemKind, bounds: Rect) {
        self.counts.add(kind, 1);
        if let Some(l) = self.lists.get_mut(kind.slot())
            && l.len() < MAX_PROBLEMS
        {
            l.push(bounds);
        }
    }
}

/// The parts of `region` (plain contours, non-zero) narrower than `2·r`: `region − open(region)`
/// with the opening shrinking by `r − slack` and growing back by `r` (see the module docs). The
/// bounds of each leftover piece above `r² / 4`.
fn narrow(region: &BezPath, r: f64, slack: f64) -> Result<Vec<Rect>, crate::ops::PathOpsError> {
    Ok(narrow_pieces(region, r, slack)?.iter().map(|s| s.bounding_box()).collect())
}

/// The leftover pieces of [`narrow`].
fn narrow_pieces(region: &BezPath, r: f64, slack: f64) -> Result<Vec<BezPath>, crate::ops::PathOpsError> {
    let shrink = r - slack;
    if !(shrink > 0.0 && r.is_finite()) || region.elements().is_empty() {
        return Ok(Vec::new());
    }
    let shrunk = crate::offset::offset(region, -shrink, StrokeJoin::Round)?;
    let shrunk = crate::clean::thin(&shrunk, slack / 2.0);
    let opened = if shrunk.elements().is_empty() { BezPath::new() } else { crate::offset::offset(&shrunk, r, StrokeJoin::Round)? };
    let left = if opened.elements().is_empty() { region.clone() } else { crate::ops::boolean_step(region, &opened, BoolOp::Subtract)? };
    let floor = r * r / 4.0;
    Ok(split_subpaths(&left).into_iter().filter(|s| s.area() > floor).collect())
}

/// The sharp paper corners of `ring` (cut on the left of travel: the cut's plain contours, outers
/// positive and holes negative), judged over `window` each way (see the module docs).
fn sharp_corners(ring: &[Point], window: f64, min_angle: f64, out: &mut Vec<(f64, Point)>) {
    let n = ring.len();
    if n < 3 {
        return;
    }
    // Arc position of every vertex, and the perimeter.
    let mut cum = Vec::with_capacity(n + 1);
    let mut s = 0.0;
    for i in 0..n {
        cum.push(s);
        let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { return };
        s += (b - a).hypot();
    }
    let perimeter = s;
    cum.push(perimeter);
    if !perimeter.is_finite() || perimeter < 4.0 * window {
        // A contour this small is a small cut or hole, not a corner.
        return;
    }
    // The point at arc position `t` (wrapped).
    let at = |t: f64| -> Option<Point> {
        let t = t.rem_euclid(perimeter);
        let i = cum.partition_point(|&c| c <= t).saturating_sub(1).min(n - 1);
        let (a, b) = (*ring.get(i)?, *ring.get((i + 1) % n)?);
        let len = cum.get(i + 1)? - cum.get(i)?;
        let f = if len > 0.0 { (t - cum.get(i)?) / len } else { 0.0 };
        Some(a.lerp(b, f.clamp(0.0, 1.0)))
    };
    let limit = min_angle.to_radians();
    let mut found: Vec<(f64, Point)> = Vec::new();
    for (i, &v) in ring.iter().enumerate() {
        let Some(&s) = cum.get(i) else { continue };
        let (Some(a), Some(b)) = (at(s - window), at(s + window)) else { continue };
        let (d1, d2): (Vec2, Vec2) = (v - a, b - v);
        let turn = d1.cross(d2).atan2(d1.dot(d2));
        // The cut is on the left: the paper's angle at v is π + the (left) turn.
        let paper = std::f64::consts::PI + turn;
        if paper.is_finite() && paper < limit {
            found.push((paper, v));
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut kept: Vec<(f64, Point)> = Vec::new();
    for (angle, v) in found {
        if kept.iter().all(|&(_, k)| (k - v).hypot() >= window) {
            kept.push((angle, v));
        }
    }
    out.extend(kept);
}

/// Check the final `cut` (plain polygonal contours in `sheet`) against `l`; `given` are problems
/// found elsewhere (the islands left floating, lattice bars too thin); the narrow checks look in
/// `zones`.
pub(crate) fn check(cut: &BezPath, sheet: Rect, given: &[Problem], l: &CheckLimits, zones: &Zones) -> Checked {
    let mut f = Found::default();
    let mut warnings = Vec::new();
    let slack = if l.slack.is_finite() { l.slack.max(0.0) } else { 0.0 };
    if !cut.elements().is_empty() {
        let paper = crate::compute::paper(sheet, cut);
        let pad = l.min_bridge.max(l.min_cut_width) + slack;
        match narrow_near(&paper, l.min_bridge / 2.0, slack, zones, pad) {
            Ok(rs) => rs.into_iter().for_each(|r| f.push(ProblemKind::NarrowPaper, r)),
            Err(_) => warnings.push("couldn't check the paper for narrow bridges".into()),
        }
        match narrow_near(cut, l.min_cut_width / 2.0, slack, zones, pad) {
            Ok(rs) => rs.into_iter().for_each(|r| f.push(ProblemKind::NarrowCut, r)),
            Err(_) => warnings.push("couldn't check the cut for narrow slivers".into()),
        }
        if l.min_area > 0.0 {
            for s in split_subpaths(cut) {
                let a = s.area();
                if a > 0.0 && a < l.min_area {
                    f.push(ProblemKind::SmallCut, s.bounding_box());
                } else if a < 0.0 && -a < l.min_area {
                    f.push(ProblemKind::SmallHole, s.bounding_box());
                }
            }
        }
        if l.min_corner_angle > 0.0 {
            let window = l.min_cut_width.max(4.0 * slack).max(sheet.width().max(sheet.height()) * 1e-6);
            if let Some(rs) = rings::rings(cut, 1e-3 * window, crate::islands::MAX_TREE_POINTS) {
                let mut corners = Vec::new();
                for r in &rs {
                    sharp_corners(r, window, l.min_corner_angle, &mut corners);
                }
                corners.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
                let h = window / 2.0;
                corners.into_iter().for_each(|(_, v)| f.push(ProblemKind::SharpCorner, Rect::new(v.x - h, v.y - h, v.x + h, v.y + h)));
            } else {
                warnings.push("the cut is too detailed to check its corners".into());
            }
        }
    }
    given.iter().for_each(|g| f.push(g.kind, g.bounds));
    let total = f.counts.total();
    // In turn from each kind, so every kind gets markers.
    let mut problems = Vec::with_capacity(total.min(MAX_PROBLEMS));
    let mut iters: Vec<_> = ProblemKind::ALL.iter().zip(f.lists.iter()).map(|(&k, l)| (k, l.iter())).collect();
    while problems.len() < MAX_PROBLEMS {
        let mut any = false;
        for (k, it) in iters.iter_mut() {
            if problems.len() >= MAX_PROBLEMS {
                break;
            }
            if let Some(&b) = it.next() {
                problems.push(Problem { kind: *k, bounds: b });
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    problems.sort_by_key(|p| p.kind);
    if total > problems.len() {
        let rest: Vec<String> = ProblemKind::ALL
            .iter()
            .filter_map(|&k| {
                let n = f.counts.get(k) - problems.iter().filter(|p| p.kind == k).count();
                (n > 0).then(|| format!("{n} {}", k.plural()))
            })
            .collect();
        warnings.push(format!("the stencil has {total} problems; {} more aren't marked ({})", total - problems.len(), rest.join(", ")));
    }
    Checked { problems, counts: f.counts, warnings }
}
