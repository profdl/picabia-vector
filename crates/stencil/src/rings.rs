//! Polygon helpers: paths flattened into closed rings, ring simplification (Ramer–Douglas–Peucker)
//! and rings written back as paths.

use vectorcraft_geom::kurbo::{self, BezPath, PathEl, Point};

/// Every subpath of `path` flattened to a closed ring of points (`tolerance` > 0, layer units),
/// open subpaths closed (a fill paints them closed), repeated points removed. Rings with fewer
/// than 3 points are left out. Stops adding points past `budget` (returns `None` then).
pub(crate) fn rings(path: &BezPath, tolerance: f64, budget: usize) -> Option<Vec<Vec<Point>>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut ring: Vec<Point> = Vec::new();
    let mut count = 0usize;
    let mut over = false;
    let flush = |ring: &mut Vec<Point>, out: &mut Vec<Vec<Point>>| {
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            out.push(std::mem::take(ring));
        } else {
            ring.clear();
        }
    };
    kurbo::flatten(path.iter(), tolerance, |el| {
        if over {
            return;
        }
        match el {
            PathEl::MoveTo(p) => {
                flush(&mut ring, &mut out);
                ring.push(p);
            }
            PathEl::LineTo(p) => {
                if ring.last() != Some(&p) {
                    ring.push(p);
                    count += 1;
                    over = count > budget;
                }
            }
            PathEl::ClosePath => flush(&mut ring, &mut out),
            // `flatten` emits lines only.
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        }
    });
    if over {
        return None;
    }
    flush(&mut ring, &mut out);
    Some(out)
}

/// `rings` as one path of closed polygons.
pub(crate) fn to_path(rings: &[Vec<Point>]) -> BezPath {
    let mut out = BezPath::new();
    for ring in rings {
        let mut pts = ring.iter();
        let Some(&first) = pts.next() else { continue };
        out.move_to(first);
        for &p in pts {
            out.line_to(p);
        }
        out.close_path();
    }
    out
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= 0.0 {
        return (p - a).hypot();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).hypot()
}

/// A closed ring simplified so no dropped point is more than `tolerance` from the kept outline
/// (Ramer–Douglas–Peucker, split at the first point and the point farthest from it). Fewer than 3
/// points left means the ring was thinner than the tolerance.
pub(crate) fn simplify_ring(ring: &[Point], tolerance: f64) -> Vec<Point> {
    let n = ring.len();
    if n < 4 || tolerance <= 0.0 {
        return ring.to_vec();
    }
    let Some(&start) = ring.first() else { return Vec::new() };
    let far = ring.iter().enumerate().fold((0, -1.0), |best, (i, p)| {
        let d = (*p - start).hypot2();
        if d > best.1 { (i, d) } else { best }
    });
    let mut keep = vec![false; n];
    if let Some(k) = keep.get_mut(0) {
        *k = true;
    }
    if let Some(k) = keep.get_mut(far.0) {
        *k = true;
    }
    // Spans (a, b) of indices, b may be n (the start again).
    let mut stack = vec![(0usize, far.0), (far.0, n)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (Some(&pa), Some(&pb)) = (ring.get(a), ring.get(b % n)) else { continue };
        let mut worst = (a, 0.0);
        for i in a + 1..b {
            let Some(&p) = ring.get(i) else { continue };
            let d = segment_distance(p, pa, pb);
            if d > worst.1 {
                worst = (i, d);
            }
        }
        if worst.1 > tolerance {
            if let Some(k) = keep.get_mut(worst.0) {
                *k = true;
            }
            stack.push((a, worst.0));
            stack.push((worst.0, b));
        }
    }
    ring.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}
