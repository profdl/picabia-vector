//! Bed split: a sheet larger than the laser's bed (either way round) becomes a grid of pieces,
//! each within the bed, neighbours overlapping by `overlap_mm`.
//!
//! Split lines (the middle of each overlap band) go on the repeat cell's edges when the cell is
//! axis-aligned and one falls within ±25% of the even spacing; otherwise at the place in that
//! window where the least cut length lies inside the band (64 samples). Each shared edge gets two
//! registration holes in the band, at the same job position on both sheets so they line up, and
//! each sheet a score line where its neighbour's edge lands.

use vectorcraft_geom::kurbo::{Point, Rect};

use crate::index::SegIndex;
use crate::laser::{LaserParams, MAX_SHEETS, Polyline, fmt_mm};

/// One piece of the job, job mm (the unkerfed sheet's top-left at 0,0).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PiecePlan {
    /// The piece, overlap bands included.
    pub rect: Rect,
    /// The piece without the bands its neighbours cover (where the label may go).
    pub free: Rect,
    /// Score lines where the neighbours' edges land.
    pub score: Vec<Polyline>,
}

/// The pieces (row by row from the top-left) and the registration holes between them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plan {
    pub pieces: Vec<PiecePlan>,
    pub holes: Vec<Point>,
}

/// Window around the even split position, as a fraction of the spacing.
const WINDOW: f64 = 0.25;
/// Split positions sampled in the window.
const SAMPLES: usize = 64;
/// The most cut edges split positions are scored on.
const MAX_SCORED_SEGMENTS: usize = 100_000;
/// Paper kept between a registration hole and the cut or the sheet edge, mm.
const HOLE_CLEAR: f64 = 1.0;

/// How many pieces a side `len` long needs when a piece may be `max` long with `ov` overlap.
fn count(len: f64, max: f64, ov: f64) -> usize {
    if len <= max {
        return 1;
    }
    let n = (len / (max - ov) - 1e-9).ceil();
    if n.is_finite() && n >= 1.0 { n as usize } else { usize::MAX }
}

/// Cut length (of `segs`) inside the band `lo..hi` along `axis` (0 = x, 1 = y).
fn band_length(segs: &[(Point, Point)], axis: usize, lo: f64, hi: f64) -> f64 {
    let c = |p: Point| if axis == 0 { p.x } else { p.y };
    let mut total = 0.0;
    for &(a, b) in segs {
        let (ca, cb) = (c(a), c(b));
        let len = (b - a).hypot();
        if ca == cb {
            if ca >= lo && ca <= hi {
                total += len;
            }
            continue;
        }
        let (t0, t1) = ((lo - ca) / (cb - ca), (hi - ca) / (cb - ca));
        let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
        if t1 > t0 {
            total += len * (t1 - t0);
        }
    }
    total
}

/// Boundaries `0 = c0 < c1 < … < cn = len` of `n` pieces along `axis`: piece `i` spans
/// `c_i − ov/2 … c_{i+1} + ov/2` (no overlap past the ends) and is at most `max` long.
fn positions(len: f64, n: usize, max: f64, ov: f64, segs: &[(Point, Point)], axis: usize, lattice: Option<(f64, f64)>) -> Vec<f64> {
    let mut c = vec![0.0];
    if n <= 1 {
        c.push(len);
        return c;
    }
    let s = len / n as f64;
    let band = ov.max(1.0) / 2.0;
    for i in 1..n {
        let prev = c.last().copied().unwrap_or(0.0);
        let even = s * i as f64;
        let lead = if i == 1 { ov / 2.0 } else { ov };
        let max_x = prev + max - lead;
        let rest = (n - i - 1) as f64 * (max - ov) + (max - ov / 2.0);
        let min_x = (len - rest).max(prev + ov);
        let ok = |x: f64| x >= min_x - 1e-9 && x <= max_x + 1e-9 && x > prev && x < len;
        let (lo, hi) = (even - WINDOW * s, even + WINDOW * s);
        let cost = |x: f64| band_length(segs, axis, x - band, x + band);
        let best = |cands: Vec<f64>| -> Option<f64> {
            cands
                .into_iter()
                .filter(|&x| ok(x))
                .map(|x| (cost(x), (x - even).abs(), x))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
                .map(|t| t.2)
        };
        let on_lattice = lattice.and_then(|(origin, step)| {
            if !(step.is_finite() && step > 1e-6 && (hi - lo) / step < 1000.0) {
                return None;
            }
            let k0 = ((lo - origin) / step).ceil() as i64;
            let k1 = ((hi - origin) / step).floor() as i64;
            best((k0..=k1).map(|k| origin + k as f64 * step).collect())
        });
        let x = on_lattice
            .or_else(|| best((0..=SAMPLES).map(|j| lo + (hi - lo) * j as f64 / SAMPLES as f64).chain(std::iter::once(even)).collect()))
            .unwrap_or_else(|| even.clamp(min_x, max_x.max(min_x)));
        c.push(x);
    }
    c.push(len);
    c
}

/// Lines `origin + k · step` along one axis.
type Lines = Option<(f64, f64)>;

/// The repeat cell's edges along x and y as (origin, step) when it is axis-aligned.
fn lattice(cell: Option<[Point; 4]>) -> (Lines, Lines) {
    let Some(q) = cell else { return (None, None) };
    if q.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return (None, None);
    }
    let bb = q.iter().fold(Rect::from_points(q[0], q[0]), |b, &p| b.union_pt(p));
    let eps = 1e-6 * bb.width().max(bb.height()).max(1.0);
    let aligned = (0..4).all(|i| {
        let d = q[(i + 1) % 4] - q[i];
        d.x.abs() <= eps || d.y.abs() <= eps
    });
    if !aligned {
        return (None, None);
    }
    let x = (bb.width() > eps).then_some((bb.x0, bb.width()));
    let y = (bb.height() > eps).then_some((bb.y0, bb.height()));
    (x, y)
}

/// The split plan for a `w` × `h` mm sheet whose cut is `cut` (rings, job mm), or `None` when it
/// needs more than [`MAX_SHEETS`] (with a warning).
pub(crate) fn plan(
    w: f64,
    h: f64,
    p: &LaserParams,
    cut: &[Vec<Point>],
    cell: Option<[Point; 4]>,
    hole_d: f64,
    warnings: &mut Vec<String>,
) -> Option<Plan> {
    let k = p.kerf_mm;
    let side = |b: f64| if b > 0.0 { b } else { f64::INFINITY };
    let (bw, bh) = (side(p.bed_w_mm), side(p.bed_h_mm));
    let (sw, sh) = (w + k, h + k);
    let single = || Plan {
        pieces: vec![PiecePlan { rect: Rect::new(0.0, 0.0, w, h), free: Rect::new(0.0, 0.0, w, h), score: Vec::new() }],
        holes: Vec::new(),
    };
    if sw <= bw && sh <= bh {
        return Some(single());
    }
    if sw <= bh && sh <= bw {
        warnings.push("the sheet fits the bed turned 90°".into());
        return Some(single());
    }
    // Pieces (without kerf) may be this long on the bed.
    let mut ov = p.overlap_mm;
    let shortest = bw.min(bh) - k;
    if ov > shortest / 4.0 {
        ov = (shortest / 4.0).max(0.0);
        warnings.push(format!("the overlap is too wide for the bed; {} mm is used", fmt_mm(ov)));
    }
    let tiles = |bx: f64, by: f64| (count(w, bx - k, ov), count(h, by - k, ov));
    let (a, b) = (tiles(bw, bh), tiles(bh, bw));
    let ((nx, ny), (mx, my)) = if a.0.saturating_mul(a.1) <= b.0.saturating_mul(b.1) { (a, (bw - k, bh - k)) } else { (b, (bh - k, bw - k)) };
    let total = nx.saturating_mul(ny);
    if total > MAX_SHEETS {
        warnings.push(format!(
            "the sheet ({} × {} mm) needs {} sheets on a {} × {} mm bed; at most {MAX_SHEETS} are made: use a bigger bed or a smaller design",
            fmt_mm(w),
            fmt_mm(h),
            if total == usize::MAX { "too many".to_string() } else { total.to_string() },
            fmt_mm(p.bed_w_mm),
            fmt_mm(p.bed_h_mm)
        ));
        return None;
    }
    // The edges the split positions are scored on; a sample of them past MAX_SCORED_SEGMENTS
    // (scoring is a comparison, so an even sample keeps the choice).
    let edges: usize = cut.iter().map(Vec::len).sum();
    let stride = edges / MAX_SCORED_SEGMENTS + 1;
    let mut segs = Vec::with_capacity(edges.min(MAX_SCORED_SEGMENTS + 1));
    let mut k = 0usize;
    for r in cut {
        let n = r.len();
        for i in 0..n {
            k += 1;
            if !k.is_multiple_of(stride) {
                continue;
            }
            if let (Some(&a), Some(&b)) = (r.get(i), r.get((i + 1) % n)) {
                segs.push((a, b));
            }
        }
    }
    let (lx, ly) = lattice(cell);
    let xs = positions(w, nx, mx, ov, &segs, 0, lx);
    let ys = positions(h, ny, my, ov, &segs, 1, ly);
    let get = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
    let half = ov / 2.0;
    let mut pieces = Vec::with_capacity(total);
    for j in 0..ny {
        for i in 0..nx {
            let (l, r, t, bt) = (i > 0, i + 1 < nx, j > 0, j + 1 < ny);
            let x0 = get(&xs, i);
            let x1 = get(&xs, i + 1);
            let y0 = get(&ys, j);
            let y1 = get(&ys, j + 1);
            let o = |has: bool| if has { half } else { 0.0 };
            let rect = Rect::new(x0 - o(l), y0 - o(t), x1 + o(r), y1 + o(bt));
            let free = Rect::new(x0 + o(l), y0 + o(t), x1 - o(r), y1 - o(bt));
            let mut score = Vec::new();
            if ov > 0.0 {
                if r {
                    score.push(vec![Point::new(x1 - half, rect.y0), Point::new(x1 - half, rect.y1)]);
                }
                if l {
                    score.push(vec![Point::new(x0 + half, rect.y0), Point::new(x0 + half, rect.y1)]);
                }
                if bt {
                    score.push(vec![Point::new(rect.x0, y1 - half), Point::new(rect.x1, y1 - half)]);
                }
                if t {
                    score.push(vec![Point::new(rect.x0, y0 + half), Point::new(rect.x1, y0 + half)]);
                }
            }
            pieces.push(PiecePlan { rect, free, score });
        }
    }
    let holes = holes(w, h, &xs, &ys, ov, hole_d, cut, warnings);
    Some(Plan { pieces, holes })
}

/// Two registration holes per shared edge, in the middle of the band, clear of the cut.
#[allow(clippy::too_many_arguments)]
fn holes(w: f64, h: f64, xs: &[f64], ys: &[f64], ov: f64, d: f64, cut: &[Vec<Point>], warnings: &mut Vec<String>) -> Vec<Point> {
    let (nx, ny) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
    if nx * ny <= 1 {
        return Vec::new();
    }
    let r = d / 2.0;
    if ov < d + 2.0 * 0.5 {
        warnings.push(format!("the overlap ({} mm) is too narrow for registration holes between the sheets", fmt_mm(ov)));
        return Vec::new();
    }
    let mut index = SegIndex::new(cut);
    let mut out: Vec<Point> = Vec::new();
    let mut missed = 0usize;
    let half = ov / 2.0;
    let get = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
    // (the split coordinate, the span along it, vertical?)
    let mut edges: Vec<(f64, f64, f64, bool)> = Vec::new();
    for i in 1..nx {
        for j in 0..ny {
            let lo = get(ys, j) + if j > 0 { half } else { 0.0 };
            let hi = get(ys, j + 1) - if j + 1 < ny { half } else { 0.0 };
            edges.push((get(xs, i), lo, hi, true));
        }
    }
    for j in 1..ny {
        for i in 0..nx {
            let lo = get(xs, i) + if i > 0 { half } else { 0.0 };
            let hi = get(xs, i + 1) - if i + 1 < nx { half } else { 0.0 };
            edges.push((get(ys, j), lo, hi, false));
        }
    }
    for (c, lo, hi, vertical) in edges {
        let span = hi - lo;
        for target in [0.25, 0.75] {
            let mut placed = false;
            for step in 0..=32 {
                let off = span * (step as f64 / 128.0);
                for sign in [1.0, -1.0] {
                    let t = lo + span * target + sign * off;
                    let p = if vertical { Point::new(c, t) } else { Point::new(t, c) };
                    let inside = p.x - r >= HOLE_CLEAR && p.y - r >= HOLE_CLEAR && p.x + r <= w - HOLE_CLEAR && p.y + r <= h - HOLE_CLEAR;
                    let along = t - r >= lo + HOLE_CLEAR && t + r <= hi - HOLE_CLEAR;
                    let apart = out.iter().all(|q| (*q - p).hypot() >= d + HOLE_CLEAR);
                    let mut clear = || index.nearest_in(p, |_| true).is_none_or(|(_, dist, _)| dist >= r + HOLE_CLEAR) && !in_cut(&mut index, p);
                    if inside && along && apart && clear() {
                        out.push(p);
                        placed = true;
                        break;
                    }
                    if step == 0 {
                        break;
                    }
                }
                if placed {
                    break;
                }
            }
            if !placed {
                missed += 1;
            }
        }
    }
    if missed > 0 {
        warnings.push(if missed == 1 {
            "1 registration hole between the sheets found no place clear of the cut and was left out".into()
        } else {
            format!("{missed} registration holes between the sheets found no place clear of the cut and were left out")
        });
    }
    out
}

/// Whether `p` lies inside the cut (an odd number of contours around it).
fn in_cut(index: &mut SegIndex, p: Point) -> bool {
    index.containing(p).len() % 2 == 1
}
