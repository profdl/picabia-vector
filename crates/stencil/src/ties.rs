//! Stage 7: automatic ties. Each island gets up to `tiesPerIsland` straight bars across the cut to
//! the paper enclosing it (the outline of the cut-out it sits in), along the shortest gaps.
//!
//! Candidates: points sampled evenly along the island's outline, each with its nearest point on
//! paper already held: the enclosing outline, or an island of the same cut-out tied before it (that
//! segment can't cross held paper: a crossing would be nearer). A candidate's cost is the gap length, times up to 4 for gaps across `tieAngle` (and
//! its opposite). A bar is the gap's rectangle `tieWidthMm` wide, run half a width into both
//! papers so the join is solid; it is refused when it touches any other contour (another island,
//! another cut-out, a cut inside the island) or its centre line crosses the island or the
//! enclosing outline again (a concave outline). Picking is greedy, cheapest first: with `tieSpread`
//! the bars keep at least half the even spacing apart along the outline and close spacing costs
//! up to twice as much; without, they only keep clear of each other. Equal costs (within 5 % of a
//! width) are ordered by SplitMix64 of `tieSeed` and the sample number: the same everywhere.

use vectorcraft_geom::kurbo::{Point, Rect, Vec2};
use vectorcraft_modifiers::splitmix64;

use crate::islands::Tree;

/// Tie settings in layer units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TieParams {
    pub width: f64,
    pub per_island: u32,
    pub spread: bool,
    /// The preferred direction (unit, layer space), if any.
    pub dir: Option<Vec2>,
    pub seed: u32,
    /// Bars stay on the sheet (a cut-out can run to its edge, where there is no paper).
    pub sheet: Rect,
}

/// The most samples along one island's outline.
const MAX_SAMPLES: usize = 384;
const MIN_SAMPLES: usize = 16;

/// A tie bar: a rectangle, counter-clockwise in a y-up view (positive area in layer space).
pub(crate) type Bar = [Point; 4];

#[derive(Clone, Copy)]
struct Candidate {
    /// Arc position along the island outline.
    at: f64,
    p: Point,
    q: Point,
    /// The held outline `q` is on.
    target: usize,
    cost: f64,
    /// Seeded order among equal costs.
    hash: u64,
    valid: Option<bool>,
}

/// Points every `perimeter / n` along `ring`, with their arc position; and the perimeter.
fn samples(ring: &[Point], spacing: f64) -> (Vec<(f64, Point)>, f64) {
    let n = ring.len();
    let edges: Vec<(Point, Point, f64)> = (0..n)
        .filter_map(|i| {
            let (a, b) = (*ring.get(i)?, *ring.get((i + 1) % n)?);
            Some((a, b, (b - a).hypot()))
        })
        .collect();
    let perimeter: f64 = edges.iter().map(|e| e.2).sum();
    if !perimeter.is_finite() || perimeter <= 0.0 {
        return (Vec::new(), 0.0);
    }
    let count = ((perimeter / spacing).ceil() as usize).clamp(MIN_SAMPLES, MAX_SAMPLES);
    let step = perimeter / count as f64;
    let mut out = Vec::with_capacity(count);
    let (mut start, mut next) = (0.0, 0.0);
    for (a, b, len) in edges {
        while next < start + len && out.len() < count {
            let t = if len > 0.0 { (next - start) / len } else { 0.0 };
            out.push((next, a.lerp(b, t)));
            next += step;
        }
        start += len;
    }
    (out, perimeter)
}

/// The bar across the gap `p`–`q`, `width` wide, run `width / 2` past both ends.
fn bar(p: Point, q: Point, width: f64) -> Option<Bar> {
    let d = q - p;
    let len = d.hypot();
    if len.is_nan() || len <= 0.0 {
        return None;
    }
    let u = d / len;
    let n = Vec2::new(-u.y, u.x) * (width / 2.0);
    let (a, b) = (p - u * (width / 2.0), q + u * (width / 2.0));
    let quad = [a + n, b + n, b - n, a - n];
    Some(if crate::islands::ring_area(&quad) < 0.0 { [quad[3], quad[2], quad[1], quad[0]] } else { quad })
}

fn arc_distance(a: f64, b: f64, perimeter: f64) -> f64 {
    let d = (a - b).abs();
    d.min(perimeter - d)
}

/// The candidate gaps of island `h` to the held paper around it, cheapest first: from each
/// sample, the nearest point on the enclosing outline `c` or on a held island in the same
/// cut-out (`held`), except islands `h` is tied to already (`linked`).
fn candidates(tree: &mut Tree, h: usize, c: usize, held: &[bool], linked: &[usize], p: &TieParams, island_no: usize) -> (Vec<Candidate>, f64) {
    let Some(ring) = tree.rings.get(h).cloned() else { return (Vec::new(), 0.0) };
    let (pts, perimeter) = samples(&ring, p.width / 2.0);
    let quantum = (p.width * 0.05).max(1e-12);
    let parent = &tree.parent;
    let take =
        |r: usize| r == c || (r != h && held.get(r).copied().unwrap_or(false) && parent.get(r).copied().flatten() == Some(c) && !linked.contains(&r));
    let mut out = Vec::with_capacity(pts.len());
    for (k, (at, pt)) in pts.into_iter().enumerate() {
        let Some((q, len, target)) = tree.index.nearest_in(pt, take) else { continue };
        if !len.is_finite() || len <= 0.0 {
            continue;
        }
        let across = p.dir.map_or(0.0, |u| {
            let c = (q - pt).dot(u) / len;
            1.0 - c * c
        });
        let cost = len * (1.0 + 3.0 * across);
        let salt = (u64::from(p.seed) << 32) ^ (island_no as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k as u64;
        out.push(Candidate { at, p: pt, q, target, cost: (cost / quantum).round() * quantum, hash: splitmix64(salt), valid: None });
    }
    (out, perimeter)
}

fn valid(tree: &mut Tree, cand: &Candidate, h: usize, p: &TieParams) -> Option<Bar> {
    let quad = bar(cand.p, cand.q, p.width)?;
    if !quad.iter().all(|&v| p.sheet.contains(v)) {
        return None;
    }
    let ends = [h, cand.target];
    if tree.index.segment_crosses(cand.p, cand.q, ends) || tree.index.quad_hits(&quad, ends) {
        return None;
    }
    Some(quad)
}

/// The next bar for one island: the cheapest valid candidate at least `sep` along the outline
/// from the bars so far (`chosen`), cost raised by up to 2x for spacing below `even`.
fn pick(tree: &mut Tree, cands: &mut [Candidate], chosen: &[f64], tp: &TieParams, ctx: (usize, f64, f64, f64)) -> Option<(usize, Bar)> {
    let (h, perimeter, sep, even) = ctx;
    let mut order: Vec<(f64, u64, usize)> = Vec::new();
    for (i, cand) in cands.iter().enumerate() {
        if cand.valid == Some(false) {
            continue;
        }
        let dmin = chosen.iter().map(|&a| arc_distance(a, cand.at, perimeter)).fold(f64::INFINITY, f64::min);
        if dmin < sep {
            continue;
        }
        let crowd = if even > 0.0 && dmin.is_finite() { 1.0 - (dmin / even).min(1.0) } else { 0.0 };
        order.push((cand.cost * (1.0 + crowd), cand.hash, i));
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, _, i) in order {
        let quad = valid(tree, cands.get(i)?, h, tp);
        if let Some(cand) = cands.get_mut(i) {
            cand.valid = Some(quad.is_some());
        }
        if let Some(q) = quad {
            return Some((i, q));
        }
    }
    None
}

/// One island being tied: its outline `h`, the outline `c` of the cut-out it sits in, its number.
#[derive(Clone, Copy)]
struct Job {
    h: usize,
    c: usize,
    no: usize,
}

/// One island's ties so far.
#[derive(Default)]
struct State {
    /// Arc positions of its bars.
    chosen: Vec<f64>,
    /// The held outlines its bars reach (an island reached once isn't reached again).
    linked: Vec<usize>,
}

/// Add bars to island `job.h` (in the cut-out outlined by `job.c`) until it has `per_island` or none fit; returns
/// whether it got any.
fn tie_one(tree: &mut Tree, job: Job, held: &[bool], st: &mut State, p: &TieParams, bars: &mut Vec<Bar>) -> bool {
    let Job { h, c, no } = job;
    let (mut cands, perimeter) = candidates(tree, h, c, held, &st.linked, p, no);
    let per = p.per_island.max(1) as usize;
    let clear = p.width * 1.5;
    let (sep, even) = if p.spread && per > 1 { ((perimeter / (2 * per) as f64).max(clear), perimeter / per as f64) } else { (clear, 0.0) };
    let mut added = false;
    while st.chosen.len() < per {
        let found = pick(tree, &mut cands, &st.chosen, p, (h, perimeter, sep, even))
            .or_else(|| if sep > clear { pick(tree, &mut cands, &st.chosen, p, (h, perimeter, clear, even)) } else { None });
        let Some((i, quad)) = found else { break };
        let Some(cand) = cands.get(i).copied() else { break };
        st.chosen.push(cand.at);
        bars.push(quad);
        added = true;
        if cand.target != c {
            // One bar per neighbouring island: drop the other gaps to it.
            st.linked.push(cand.target);
            for k in cands.iter_mut().filter(|k| k.target == cand.target) {
                k.valid = Some(false);
            }
        }
    }
    added
}

/// Tie `islands` (rings of `tree`, outermost first): the bars, and how many each island got.
///
/// Rounds, outside in: each round ties the islands that can reach paper already held (the
/// outline of the cut-out they sit in, or an island of the same cut-out tied in an earlier round)
/// with a bar across cut only; an island boxed in by others is tied once one of them is held.
/// Islands short of `tiesPerIsland` keep trying in later rounds. Stops when a round adds nothing.
pub(crate) fn tie(tree: &mut Tree, islands: &[usize], p: &TieParams) -> (Vec<Bar>, Vec<u32>) {
    let (bars, counts, _) = tie_owned(tree, islands, p);
    (bars, counts)
}

/// [`tie`], with the island each bar holds (its position in `islands`).
pub(crate) fn tie_owned(tree: &mut Tree, islands: &[usize], p: &TieParams) -> (Vec<Bar>, Vec<u32>, Vec<usize>) {
    let mut bars = Vec::new();
    let mut owners = Vec::new();
    let mut held = vec![false; tree.rings.len()];
    let mut states: Vec<State> = islands.iter().map(|_| State::default()).collect();
    let per = p.per_island.max(1) as usize;
    loop {
        let mut newly = Vec::new();
        for (no, &h) in islands.iter().enumerate() {
            let Some(st) = states.get_mut(no) else { continue };
            if st.chosen.len() >= per {
                continue;
            }
            let Some(c) = tree.parent.get(h).copied().flatten() else { continue };
            if tie_one(tree, Job { h, c, no }, &held, st, p, &mut bars) {
                newly.push(h);
            }
            owners.resize(bars.len(), no);
        }
        if newly.is_empty() {
            break;
        }
        for h in newly {
            if let Some(x) = held.get_mut(h) {
                *x = true;
            }
        }
    }
    let counts = states.iter().map(|s| u32::try_from(s.chosen.len()).unwrap_or(u32::MAX)).collect();
    (bars, counts, owners)
}
