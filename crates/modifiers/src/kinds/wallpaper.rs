//! `wallpaper`: the 17 plane crystallographic (wallpaper) groups.
//!
//! A group is a lattice (two translation vectors) plus the motif transforms inside one lattice
//! cell. Copy `k` is cell `k / ops` (row-major: column varies fastest) and op `k % ops`, so copy 0
//! (cell 0, op 0) is the identity: the layer itself. The symmetry origin (rotation centres on it,
//! mirror axes through it) is the top-left corner of the layer's bounds plus `originX`/`originY`;
//! the layer is the motif.
//!
//! The ops follow the standard descriptions of the groups (coordinates as fractions of the cell
//! basis, the "general position" lists of the International Tables), written out here as 2x3
//! matrices `x' = a x + b y + t`. The lattice per group:
//!
//! - oblique (`p1`, `p2`): `a = (w, 0)`, `b = h (cos θ, sin θ)`.
//! - rectangular (`pm`, `pg`, `pmm`, `pmg`, `pgg`): `a = (w, 0)`, `b = (0, h)`.
//! - centred rectangular (`cm`, `cmm`): the ops use the conventional `w x h` rectangle; the lattice
//!   is its primitive one, `a = (w, 0)`, `b = (w/2, h/2)`, so a cell holds 2 (`cm`) or 4 (`cmm`)
//!   motifs, not the 4 or 8 of the conventional cell.
//! - square (`p4`, `p4m`, `p4g`): `a = (w, 0)`, `b = (0, w)`; `cellHeight` is ignored.
//! - hexagonal (`p3`, `p3m1`, `p31m`, `p6`, `p6m`): `a = (w, 0)`, `b = w (1/2, √3/2)`; `w` is the
//!   lattice constant, `cellHeight` is ignored.

use vectorcraft_geom::{Affine, Point, Vec2};

use super::{Clamp, Ctx, Stage, StageResult};
use crate::MAX_INSTANCES_PER_NODE;
use crate::eval::{Guide, Repeat};
use crate::model::{Wallpaper, WallpaperGroup};

/// The most cells along one side.
pub(crate) const MAX_CELLS: u32 = 100;

type Row = [f64; 3];

/// `x' = x[0] x + x[1] y + x[2]`, `y' = y[0] x + y[1] y + y[2]` in the group's fractional basis.
fn op(x: Row, y: Row) -> Affine {
    Affine::new([x[0], y[0], x[1], y[1], x[2], y[2]])
}

const H: f64 = 0.5;

/// The motif transforms of `g` in its fractional basis; the first is the identity.
fn fractional_ops(g: WallpaperGroup) -> Vec<Affine> {
    use WallpaperGroup::*;
    let id = op([1., 0., 0.], [0., 1., 0.]);
    let c2 = op([-1., 0., 0.], [0., -1., 0.]);
    let mx = op([-1., 0., 0.], [0., 1., 0.]); // x -> -x
    let my = op([1., 0., 0.], [0., -1., 0.]); // y -> -y
    let r4 = op([0., -1., 0.], [1., 0., 0.]); // (x,y) -> (-y, x)
    let r4i = op([0., 1., 0.], [-1., 0., 0.]);
    let diag = op([0., 1., 0.], [1., 0., 0.]); // (y, x)
    let adiag = op([0., -1., 0.], [-1., 0., 0.]); // (-y, -x)
    // Hexagonal basis (120 degrees between a and b).
    let r3 = op([0., -1., 0.], [1., -1., 0.]); // (-y, x-y)
    let r3i = op([-1., 1., 0.], [-1., 0., 0.]); // (-x+y, -x)
    let r6 = op([1., -1., 0.], [1., 0., 0.]); // (x-y, x)
    let r6i = op([0., 1., 0.], [-1., 1., 0.]); // (y, -x+y)
    // p3m1 mirrors, then p31m mirrors.
    let m1 = [adiag, op([-1., 1., 0.], [0., 1., 0.]), op([1., 0., 0.], [1., -1., 0.])];
    let m2 = [diag, op([1., -1., 0.], [0., -1., 0.]), op([-1., 0., 0.], [-1., 1., 0.])];
    match g {
        P1 => vec![id],
        P2 => vec![id, c2],
        Pm | Cm => vec![id, mx],
        Pg => vec![id, op([-1., 0., 0.], [0., 1., H])],
        Pmm | Cmm => vec![id, mx, my, c2],
        Pmg => vec![id, c2, op([-1., 0., H], [0., 1., 0.]), op([1., 0., H], [0., -1., 0.])],
        Pgg => vec![id, c2, op([-1., 0., H], [0., 1., H]), op([1., 0., H], [0., -1., H])],
        P4 => vec![id, r4, c2, r4i],
        P4m => vec![id, r4, c2, r4i, mx, my, diag, adiag],
        P4g => vec![
            id,
            c2,
            r4,
            r4i,
            op([-1., 0., H], [0., 1., H]),
            op([1., 0., H], [0., -1., H]),
            op([0., 1., H], [1., 0., H]),
            op([0., -1., H], [-1., 0., H]),
        ],
        P3 => vec![id, r3, r3i],
        P3m1 => vec![id, r3, r3i, m1[0], m1[1], m1[2]],
        P31m => vec![id, r3, r3i, m2[0], m2[1], m2[2]],
        P6 => vec![id, r3, r3i, c2, r6i, r6],
        P6m => vec![id, r3, r3i, c2, r6i, r6, m1[0], m1[1], m1[2], m2[0], m2[1], m2[2]],
    }
}

/// The lattice basis `(a, b)` of `g` for a `w x h` cell and oblique angle `angle` (degrees).
pub(crate) fn lattice(g: WallpaperGroup, w: f64, h: f64, angle: f64) -> ([f64; 2], [f64; 2]) {
    use WallpaperGroup::*;
    match g {
        P1 | P2 => {
            let t = angle.to_radians();
            ([w, 0.0], [h * t.cos(), h * t.sin()])
        }
        Pm | Pg | Pmm | Pmg | Pgg => ([w, 0.0], [0.0, h]),
        Cm | Cmm => ([w, 0.0], [w / 2.0, h / 2.0]),
        P4 | P4m | P4g => ([w, 0.0], [0.0, w]),
        P3 | P3m1 | P31m | P6 | P6m => ([w, 0.0], [w / 2.0, w * 3f64.sqrt() / 2.0]),
    }
}

/// The basis the fractional ops are written in: the lattice for most groups, the conventional
/// rectangle for the centred ones, the 120-degree basis for the hexagonal ones.
fn ops_basis(g: WallpaperGroup, w: f64, h: f64, angle: f64) -> Affine {
    use WallpaperGroup::*;
    let (a, b) = match g {
        Cm | Cmm => ([w, 0.0], [0.0, h]),
        P3 | P3m1 | P31m | P6 | P6m => ([w, 0.0], [-w / 2.0, w * 3f64.sqrt() / 2.0]),
        _ => lattice(g, w, h, angle),
    };
    Affine::new([a[0], a[1], b[0], b[1], 0.0, 0.0])
}

/// The motif transforms of `g` about the symmetry origin (0, 0), in points; the first is the
/// identity. Their count is the number of motif copies per lattice cell.
pub(crate) fn group_ops(g: WallpaperGroup, w: f64, h: f64, angle: f64) -> Vec<Affine> {
    let b = ops_basis(g, w, h, angle);
    let inv = b.inverse();
    fractional_ops(g).into_iter().map(|m| b * m * inv).collect()
}

/// The clamped cell geometry of a wallpaper row: what [`stage`] and [`repeat`] both use.
struct Cell {
    cols: usize,
    rows: usize,
    w: f64,
    h: f64,
    angle: f64,
    /// The symmetry origin (a lattice point), parent space.
    origin: Point,
}

fn cell_of(p: &Wallpaper, ctx: &Ctx, c: &mut Clamp<'_>) -> Cell {
    let cols = clamp_cells(c, "cols", p.cols);
    let rows = clamp_cells(c, "rows", p.rows);
    let w = c.within("cellWidth", p.cell_width, 0.001, crate::MAX_LENGTH, 100.0);
    let h = c.within("cellHeight", p.cell_height, 0.001, crate::MAX_LENGTH, 100.0);
    let angle = c.within("angle", p.angle, 1.0, 179.0, 90.0);
    let origin = Point::new(ctx.bounds.x0 + c.length("originX", p.origin_x), ctx.bounds.y0 + c.length("originY", p.origin_y));
    Cell { cols, rows, w, h, angle, origin }
}

/// The row's translation lattice: its [`lattice`] basis at the symmetry origin, whatever `cols`
/// and `rows` are (the pattern is periodic along both vectors).
pub(crate) fn repeat(p: &Wallpaper, ctx: &Ctx) -> Repeat {
    let Cell { w, h, angle, origin, .. } = cell_of(p, ctx, &mut Clamp { warnings: &mut Vec::new() });
    let (a, b) = lattice(p.group, w, h, angle);
    Repeat { origin, a: Vec2::new(a[0], a[1]), b: Vec2::new(b[0], b[1]) }
}

pub(crate) fn stage(p: &Wallpaper, ctx: &Ctx) -> StageResult {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let Cell { cols, rows, w, h, angle, origin } = cell_of(p, ctx, &mut c);
    let per_cell = fractional_ops(p.group).len();
    let total = rows.saturating_mul(cols).saturating_mul(per_cell);
    // Checked before allocating anything per copy.
    if total.saturating_mul(ctx.n_prev) > MAX_INSTANCES_PER_NODE {
        return StageResult { stage: Stage::TooMany(total), guides: Vec::new(), warnings };
    }
    let ops = group_ops(p.group, w, h, angle);
    let (a, b) = lattice(p.group, w, h, angle);
    let to_origin = Affine::translate(origin.to_vec2());
    let from_origin = Affine::translate(-origin.to_vec2());
    let mut copies = Vec::with_capacity(total);
    for j in 0..rows {
        for i in 0..cols {
            let cell = Affine::translate((a[0] * i as f64 + b[0] * j as f64, a[1] * i as f64 + b[1] * j as f64));
            for o in &ops {
                copies.push(cell * to_origin * *o * from_origin);
            }
        }
    }
    let guides = lattice_guides(origin, a, b, &copies, &ctx.bounds);
    StageResult { stage: Stage::Array(copies), guides, warnings }
}

/// The most lattice lines drawn along one basis vector.
const MAX_GUIDE_LINES: usize = 202;

/// The lattice lines through `origin` (the symmetry origin) along `a` and `b`, spanning exactly
/// the copies: the lines reach from the first to the last lattice line that the transformed
/// layer `bounds` touch, so reflections that land before the origin are enclosed too.
fn lattice_guides(origin: Point, a: [f64; 2], b: [f64; 2], copies: &[Affine], bounds: &vectorcraft_geom::Rect) -> Vec<Guide> {
    let d = a[0] * b[1] - a[1] * b[0];
    if !d.is_finite() || d.abs() < 1e-12 {
        return Vec::new();
    }
    // Lattice coordinates (u, v) of a point: p - origin = u a + v b.
    let uv = |p: Point| ((p.x - origin.x) * b[1] - (p.y - origin.y) * b[0], a[0] * (p.y - origin.y) - a[1] * (p.x - origin.x));
    let corners =
        [Point::new(bounds.x0, bounds.y0), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)];
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for t in copies {
        for c in corners {
            let (u, v) = uv(*t * c);
            let (u, v) = (u / d, v / d);
            if u.is_finite() && v.is_finite() {
                u0 = u0.min(u);
                u1 = u1.max(u);
                v0 = v0.min(v);
                v1 = v1.max(v);
            }
        }
    }
    if u0 > u1 || v0 > v1 {
        return Vec::new();
    }
    const EPS: f64 = 1e-6;
    // Whole lattice lines enclosing the span; a degenerate span (a flat layer) still gets a cell.
    let span = |lo: f64, hi: f64| -> (i64, i64) {
        let (l, h) = ((lo + EPS).floor(), (hi - EPS).ceil());
        let (l, h) = (l.clamp(-1e6, 1e6) as i64, h.clamp(-1e6, 1e6) as i64);
        (l, h.max(l + 1))
    };
    let ((ku0, ku1), (kv0, kv1)) = (span(u0, u1), span(v0, v1));
    let at = |u: i64, v: i64| Point::new(origin.x + a[0] * u as f64 + b[0] * v as f64, origin.y + a[1] * u as f64 + b[1] * v as f64);
    let mut guides = Vec::new();
    for v in (kv0..=kv1).take(MAX_GUIDE_LINES) {
        guides.push(Guide::Line { from: at(ku0, v), to: at(ku1, v) });
    }
    for u in (ku0..=ku1).take(MAX_GUIDE_LINES) {
        guides.push(Guide::Line { from: at(u, kv0), to: at(u, kv1) });
    }
    guides
}

fn clamp_cells(c: &mut Clamp<'_>, name: &str, v: u32) -> usize {
    if v > MAX_CELLS {
        c.warnings.push(format!("`{name}` is limited to {MAX_CELLS}"));
    }
    usize::try_from(v.clamp(1, MAX_CELLS)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_geom::{Affine, Rect};

    use super::*;
    use crate::{Output, Stack, evaluate, parse};

    const B: Rect = Rect::new(10.0, 20.0, 30.0, 35.0);
    const COUNTS: [usize; 17] = [1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 8, 8, 3, 6, 6, 6, 12];

    fn eval(m: serde_json::Value) -> Output {
        let s: Stack = parse(&json!({"v": 1, "stack": [m]}).to_string()).unwrap();
        evaluate(&s, B)
    }

    fn det(a: Affine) -> f64 {
        let c = a.as_coeffs();
        c[0] * c[3] - c[1] * c[2]
    }

    fn orthonormal(a: Affine) -> bool {
        let c = a.as_coeffs();
        let (c0, c1, c2, c3) = (c[0], c[1], c[2], c[3]);
        (c0 * c0 + c1 * c1 - 1.0).abs() < 1e-9 && (c2 * c2 + c3 * c3 - 1.0).abs() < 1e-9 && (c0 * c2 + c1 * c3).abs() < 1e-9
    }

    #[test]
    fn copies_per_cell() {
        for (g, n) in WallpaperGroup::ALL.iter().zip(COUNTS) {
            assert_eq!(group_ops(*g, 100.0, 80.0, 70.0).len(), n, "{g:?}");
            let out = eval(json!({"id": "w", "kind": "wallpaper", "group": g, "cols": 2, "rows": 3}));
            assert_eq!(out.instances.len(), 6 * n, "{g:?}");
            assert!(out.warnings.is_empty(), "{:?}", out.warnings);
            assert_eq!(out.instances[0].transform, Affine::IDENTITY);
        }
    }

    #[test]
    fn every_transform_is_an_isometry() {
        for g in WallpaperGroup::ALL {
            for (w, h, ang) in [(100.0, 100.0, 90.0), (120.0, 70.0, 60.0)] {
                for o in group_ops(g, w, h, ang) {
                    assert!((det(o).abs() - 1.0).abs() < 1e-9, "{g:?} {o:?}");
                    assert!(orthonormal(o), "{g:?} {o:?}");
                }
            }
            let out = eval(json!({"id": "w", "kind": "wallpaper", "group": g, "cellWidth": 90, "cellHeight": 50, "angle": 65}));
            for i in &out.instances {
                assert!((det(i.transform).abs() - 1.0).abs() < 1e-9 && orthonormal(i.transform), "{g:?}");
            }
        }
    }

    #[test]
    fn reflections_exist_exactly_in_the_groups_that_have_them() {
        use WallpaperGroup::*;
        let with = [Pm, Pg, Cm, Pmm, Pmg, Pgg, Cmm, P4m, P4g, P3m1, P31m, P6m];
        for g in WallpaperGroup::ALL {
            let has = group_ops(g, 100.0, 100.0, 90.0).iter().any(|o| det(*o) < 0.0);
            assert_eq!(has, with.contains(&g), "{g:?}");
        }
    }

    /// The ops together with the lattice translations close under composition.
    #[test]
    fn ops_form_a_group_modulo_the_lattice() {
        for g in WallpaperGroup::ALL {
            let (w, h, ang) = (100.0, 70.0, 75.0);
            let ops = group_ops(g, w, h, ang);
            let (a, b) = lattice(g, w, h, ang);
            // Lattice coordinates of a point: solve p = u a + v b.
            let d = a[0] * b[1] - a[1] * b[0];
            let is_lattice = |p: vectorcraft_geom::Point| {
                let (u, v) = ((p.x * b[1] - p.y * b[0]) / d, (a[0] * p.y - a[1] * p.x) / d);
                (u - u.round()).abs() < 1e-9 && (v - v.round()).abs() < 1e-9
            };
            for x in &ops {
                for y in &ops {
                    let z = *x * *y;
                    let found = ops.iter().any(|o| {
                        let l = z.as_coeffs();
                        let m = o.as_coeffs();
                        (0..4).all(|i| (l[i] - m[i]).abs() < 1e-9) && is_lattice(vectorcraft_geom::Point::new(l[4] - m[4], l[5] - m[5]))
                    });
                    assert!(found, "{g:?} not closed");
                }
            }
        }
    }

    #[test]
    fn cells_are_lattice_translates() {
        let out = eval(json!({"kind": "wallpaper", "group": "p1", "cols": 3, "rows": 2, "cellWidth": 50, "cellHeight": 40}));
        let c = out.instances[4].transform.as_coeffs(); // row 1, col 1
        assert!((c[4] - 50.0).abs() < 1e-9 && (c[5] - 40.0).abs() < 1e-9);
        // The oblique angle slants the second axis.
        let out = eval(json!({"kind": "wallpaper", "group": "p2", "cols": 1, "rows": 2, "cellWidth": 50, "cellHeight": 40, "angle": 60}));
        let c = out.instances[2].transform.as_coeffs();
        assert!((c[4] - 20.0).abs() < 1e-9 && (c[5] - 40.0 * 3f64.sqrt() / 2.0).abs() < 1e-9);
    }

    #[test]
    fn mirror_axis_runs_through_the_bounds_corner() {
        let out = eval(json!({"kind": "wallpaper", "group": "pm", "cols": 1, "rows": 1}));
        let m = out.instances[1].transform;
        // x = 10 is the axis: the corner is fixed and x = 30 goes to x = -10.
        assert!((m * vectorcraft_geom::Point::new(10.0, 25.0) - vectorcraft_geom::Point::new(10.0, 25.0)).hypot() < 1e-9);
        assert!((m * vectorcraft_geom::Point::new(30.0, 25.0) - vectorcraft_geom::Point::new(-10.0, 25.0)).hypot() < 1e-9);
    }

    #[test]
    fn caps() {
        // 100 x 100 x 12 would be 120 000: nothing is produced, with a warning.
        let out = eval(json!({"id": "w", "kind": "wallpaper", "group": "p6m", "cols": 100, "rows": 100}));
        assert!(out.instances.len() <= crate::MAX_INSTANCES_PER_NODE);
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].modifier.as_deref(), Some("w"));
        // 100 x 100 x 1 = 10 000 fits.
        let out = eval(json!({"kind": "wallpaper", "group": "p1", "cols": 100, "rows": 100}));
        assert_eq!(out.instances.len(), 10_000);
        assert_eq!(out.guides.len(), 202);
        // Past 100 cells a side is limited.
        let out = eval(json!({"kind": "wallpaper", "group": "p1", "cols": 500, "rows": 1}));
        assert_eq!(out.instances.len(), 100);
        assert_eq!(out.warnings.len(), 1);
        // Stacked after another array the product is capped too.
        let s = parse(
            &json!({"v": 1, "stack": [{"id": "a", "kind": "linearArray", "count": 200}, {"id": "w", "kind": "wallpaper", "group": "p4", "cols": 10, "rows": 10}]})
                .to_string(),
        )
        .unwrap();
        let out = evaluate(&s, B);
        assert_eq!(out.instances.len(), 200);
    }

    #[test]
    fn nan_and_huge_params_are_clamped() {
        let huge = json!({"kind": "wallpaper", "group": "cmm", "cols": 2, "rows": 2, "cellWidth": 1e300, "cellHeight": -5, "angle": 1e300, "originX": 1e300, "originY": -1e300});
        let out = eval(huge);
        assert_eq!(out.instances.len(), 16);
        assert!(out.instances.iter().all(|i| i.transform.is_finite()));
        assert!(!out.warnings.is_empty());
        let mut m = crate::Modifier::new("m1", "wallpaper").unwrap();
        if let crate::ModifierKind::Wallpaper(p) = &mut m.kind {
            p.cell_width = f64::NAN;
            p.cell_height = f64::INFINITY;
            p.angle = f64::NAN;
            p.origin_x = f64::NAN;
        }
        let mut s = Stack::default();
        s.modifiers.push(m);
        let out = evaluate(&s, B);
        assert_eq!(out.instances.len(), 9 * 8);
        assert!(out.instances.iter().all(|i| i.transform.is_finite()));
    }

    #[test]
    fn json_round_trip_and_bad_group() {
        let m = crate::Modifier::new("m1", "wallpaper").unwrap();
        let v = m.to_value();
        assert_eq!(v["group"], "p4m");
        assert_eq!(crate::Modifier::from_value(&v).unwrap(), m);
        for g in WallpaperGroup::ALL {
            let v = serde_json::to_value(g).unwrap();
            assert_eq!(serde_json::from_value::<WallpaperGroup>(v).unwrap(), g);
        }
        // An unknown group name doesn't read: the modifier is kept as unknown and ignored.
        let m = crate::Modifier::from_value(&json!({"id": "x", "kind": "wallpaper", "group": "p5"})).unwrap();
        assert!(matches!(m.kind, crate::ModifierKind::Other { .. }));
        assert_eq!(m.to_value()["group"], "p5");
    }

    // ----- every input of every group -----

    fn wp(g: WallpaperGroup, extra: serde_json::Value) -> Output {
        let mut m = json!({"id": "w", "kind": "wallpaper", "group": g, "cols": 2, "rows": 2, "cellWidth": 100, "cellHeight": 70, "angle": 75, "originX": 0, "originY": 0});
        if let (Some(m), Some(e)) = (m.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                m.insert(k.clone(), v.clone());
            }
        }
        eval(m)
    }

    fn same(a: &Output, b: &Output) -> bool {
        a.instances.len() == b.instances.len()
            && a.instances.iter().zip(&b.instances).all(|(x, y)| {
                let (p, q) = (x.transform.as_coeffs(), y.transform.as_coeffs());
                (0..6).all(|i| (p[i] - q[i]).abs() < 1e-9)
            })
    }

    fn pt(x: f64, y: f64) -> vectorcraft_geom::Point {
        vectorcraft_geom::Point::new(x, y)
    }

    fn near(a: vectorcraft_geom::Point, b: vectorcraft_geom::Point) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn cols_and_rows_set_the_cell_count_for_every_group() {
        for (g, n) in WallpaperGroup::ALL.iter().zip(COUNTS) {
            for (c, r) in [(1, 1), (4, 1), (1, 5), (3, 2)] {
                let out = wp(*g, json!({"cols": c, "rows": r}));
                assert_eq!(out.instances.len(), c * r * n, "{g:?} {c}x{r}");
                // Row-major with columns fastest: cell k starts at copy k * n, translated by its
                // lattice vector.
                let (a, b) = lattice(*g, 100.0, 70.0, 75.0);
                for j in 0..r {
                    for i in 0..c {
                        let t = out.instances[(j * c + i) * n].transform.as_coeffs();
                        assert!((t[4] - (a[0] * i as f64 + b[0] * j as f64)).abs() < 1e-9, "{g:?} cell {i},{j}");
                        assert!((t[5] - (a[1] * i as f64 + b[1] * j as f64)).abs() < 1e-9, "{g:?} cell {i},{j}");
                    }
                }
            }
        }
    }

    #[test]
    fn cell_width_scales_the_lattice_of_every_group() {
        for g in WallpaperGroup::ALL {
            let a = wp(g, json!({"cellWidth": 100}));
            let b = wp(g, json!({"cellWidth": 150}));
            assert!(!same(&a, &b), "{g:?}: cellWidth changes the output");
            let n = group_ops(g, 100.0, 70.0, 75.0).len();
            // The step to the next column is (w, 0) for every group.
            let t = b.instances[n].transform.as_coeffs();
            assert!((t[4] - 150.0).abs() < 1e-9 && t[5].abs() < 1e-9, "{g:?}");
        }
    }

    #[test]
    fn cell_height_applies_exactly_where_the_lattice_uses_it() {
        for g in WallpaperGroup::ALL {
            let a = wp(g, json!({"cellHeight": 70}));
            let b = wp(g, json!({"cellHeight": 120}));
            assert_eq!(!same(&a, &b), g.uses_cell_height(), "{g:?}");
        }
        // Rectangular and centred groups: the second lattice vector's length follows it.
        let n = |g| group_ops(g, 100.0, 70.0, 90.0).len();
        let out = wp(WallpaperGroup::Pmm, json!({"cols": 1, "rows": 2, "cellHeight": 120}));
        assert!((out.instances[n(WallpaperGroup::Pmm)].transform.as_coeffs()[5] - 120.0).abs() < 1e-9);
        let out = wp(WallpaperGroup::Cmm, json!({"cols": 1, "rows": 2, "cellHeight": 120}));
        let t = out.instances[n(WallpaperGroup::Cmm)].transform.as_coeffs();
        assert!((t[4] - 50.0).abs() < 1e-9 && (t[5] - 60.0).abs() < 1e-9);
        // The mirror axes of the centred groups keep the conventional rectangle: cmm's second
        // mirror is the horizontal axis through the origin, whatever the height.
        let out = wp(WallpaperGroup::Cmm, json!({"cellHeight": 120}));
        let m = out.instances[2].transform;
        assert!(near(m * pt(10.0, 20.0), pt(10.0, 20.0)));
        assert!(near(m * pt(14.0, 29.0), pt(14.0, 11.0)));
    }

    #[test]
    fn angle_applies_exactly_to_the_oblique_groups() {
        for g in WallpaperGroup::ALL {
            let a = wp(g, json!({"angle": 60}));
            let b = wp(g, json!({"angle": 110}));
            assert_eq!(!same(&a, &b), g.uses_angle(), "{g:?}");
        }
        for g in [WallpaperGroup::P1, WallpaperGroup::P2] {
            let out = wp(g, json!({"cols": 1, "rows": 2, "cellWidth": 100, "cellHeight": 40, "angle": 60}));
            let n = if g == WallpaperGroup::P1 { 1 } else { 2 };
            let t = out.instances[n].transform.as_coeffs();
            assert!((t[4] - 20.0).abs() < 1e-9 && (t[5] - 40.0 * 3f64.sqrt() / 2.0).abs() < 1e-9, "{g:?}");
        }
        // Rectangular lattices stay at a right angle whatever `angle` says.
        let out = wp(WallpaperGroup::Pmg, json!({"angle": 33, "cols": 1, "rows": 2}));
        assert!(out.instances[4].transform.as_coeffs()[4].abs() < 1e-9);
    }

    /// Moving the origin by Δ moves the whole symmetry pattern by Δ: the copy transforms are
    /// conjugated by the shift, for every group; the pattern really moves for all but p1.
    #[test]
    fn origin_moves_the_symmetry_elements_of_every_group() {
        let d = (7.0, -4.0);
        for g in WallpaperGroup::ALL {
            let base = wp(g, json!({}));
            let moved = wp(g, json!({"originX": d.0, "originY": d.1}));
            assert_eq!(base.instances.len(), moved.instances.len());
            for (x, y) in base.instances.iter().zip(&moved.instances) {
                for p in [pt(10.0, 20.0), pt(33.0, 41.0)] {
                    let want = x.transform * p + kurbo_vec(d);
                    assert!(near(y.transform * (p + kurbo_vec(d)), want), "{g:?}");
                }
            }
            assert_eq!(!same(&base, &moved), g.uses_origin(), "{g:?}");
        }
    }

    fn kurbo_vec(d: (f64, f64)) -> vectorcraft_geom::Vec2 {
        vectorcraft_geom::Vec2::new(d.0, d.1)
    }

    /// Where the symmetry elements of each group sit, from the standard descriptions, for cell
    /// `w x h` (the layer's corner `(10, 20)` is the origin).
    #[test]
    fn symmetry_elements_sit_where_crystallography_puts_them() {
        use WallpaperGroup::*;
        let (w, h) = (100.0, 70.0);
        let at = |g: WallpaperGroup, k: usize, p: vectorcraft_geom::Point| {
            let ops = group_ops(g, w, h, 90.0);
            let o = pt(10.0, 20.0);
            Affine::translate(o.to_vec2()) * ops[k] * Affine::translate(-o.to_vec2()) * p
        };
        let (x, y) = (3.0, 5.0); // an offset from the origin
        let p = pt(10.0 + x, 20.0 + y);
        let q = |dx: f64, dy: f64| pt(10.0 + dx, 20.0 + dy);
        // Rotations about the origin.
        assert!(near(at(P2, 1, p), q(-x, -y)));
        assert!(near(at(P4, 1, p), q(-y, x)));
        assert!(near(at(P4, 3, p), q(y, -x)));
        // Mirrors: pm/pmm/cm/cmm have vertical (and horizontal) axes through the origin.
        assert!(near(at(Pm, 1, p), q(-x, y)));
        assert!(near(at(Cm, 1, p), q(-x, y)));
        assert!(near(at(Pmm, 1, p), q(-x, y)) && near(at(Pmm, 2, p), q(x, -y)));
        assert!(near(at(Cmm, 1, p), q(-x, y)) && near(at(Cmm, 2, p), q(x, -y)));
        // p4m adds the diagonal mirrors.
        assert!(near(at(P4m, 6, p), q(y, x)) && near(at(P4m, 7, p), q(-y, -x)));
        // Glides: pg reflects in the vertical axis and slides half a cell down it.
        assert!(near(at(Pg, 1, p), q(-x, y + h / 2.0)));
        // pmg: vertical mirrors a quarter cell out, half-cell glides in the horizontal axis.
        assert!(near(at(Pmg, 2, p), q(w / 2.0 - x, y)));
        assert!(near(at(Pmg, 3, p), q(x + w / 2.0, -y)));
        // pgg: glides in both directions, half a cell along each.
        assert!(near(at(Pgg, 2, p), q(w / 2.0 - x, y + h / 2.0)));
        assert!(near(at(Pgg, 3, p), q(x + w / 2.0, h / 2.0 - y)));
        // p4g: mirrors half a (square) cell out, and the diagonal glides.
        assert!(near(at(P4g, 4, p), q(w / 2.0 - x, y + w / 2.0)));
        assert!(near(at(P4g, 6, p), q(y + w / 2.0, x + w / 2.0)));
        // Hexagonal groups: the rotation centres at the origin turn by 120 and 60 degrees.
        let turn = |g: WallpaperGroup, k: usize| {
            let r = at(g, k, q(10.0, 0.0)) - q(0.0, 0.0);
            r.atan2().to_degrees().rem_euclid(360.0)
        };
        assert!((turn(P3, 1) - 120.0).abs() < 1e-6 || (turn(P3, 1) - 240.0).abs() < 1e-6);
        assert!((turn(P6, 5) - 60.0).abs() < 1e-6 || (turn(P6, 5) - 300.0).abs() < 1e-6);
        assert!((turn(P6, 3) - 180.0).abs() < 1e-6);
        // The hexagonal lattice is w wide with 60 degrees between its vectors.
        let (a, b) = lattice(P6m, w, h, 90.0);
        assert!((a[0] - w).abs() < 1e-9 && (b[0] - w / 2.0).abs() < 1e-9 && (b[1] - w * 3f64.sqrt() / 2.0).abs() < 1e-9);
        // The square lattice is w by w.
        let (a, b) = lattice(P4g, w, h, 90.0);
        assert!((a[0] - w).abs() < 1e-9 && b[0].abs() < 1e-9 && (b[1] - w).abs() < 1e-9);
    }

    /// No two copies of a generic point coincide: no group's ops are duplicated or overlap
    /// modulo the lattice, for any cell shape.
    #[test]
    fn copies_of_a_generic_point_are_all_distinct() {
        for g in WallpaperGroup::ALL {
            for (w, h, ang) in [(100.0, 70.0, 90.0), (80.0, 120.0, 60.0), (50.0, 50.0, 110.0)] {
                let ops = group_ops(g, w, h, ang);
                let (a, b) = lattice(g, w, h, ang);
                let d = a[0] * b[1] - a[1] * b[0];
                let p = pt(0.137 * w, 0.211 * h);
                for (i, x) in ops.iter().enumerate() {
                    for y in &ops[..i] {
                        let diff = *x * p - *y * p;
                        let (u, v) = ((diff.x * b[1] - diff.y * b[0]) / d, (a[0] * diff.y - a[1] * diff.x) / d);
                        let lattice_vec = (u - u.round()).abs() < 1e-7 && (v - v.round()).abs() < 1e-7;
                        assert!(!lattice_vec, "{g:?} {w}x{h} {ang}: two ops put a generic point on the same spot");
                    }
                }
            }
        }
    }

    /// The guides are lattice lines that enclose the copies of a cell-sized motif, touch their
    /// outer edge and don't run more than a cell past it.
    #[test]
    fn guides_line_up_with_the_copies_of_every_group() {
        for g in WallpaperGroup::ALL {
            for (w, h, ang) in [(30.0, 35.0, 90.0), (30.0, 35.0, 65.0)] {
                let out = wp(g, json!({"cols": 3, "rows": 2, "cellWidth": w, "cellHeight": h, "angle": ang, "originX": 3, "originY": 2}));
                let (a, b) = lattice(g, w, h, ang);
                let d = a[0] * b[1] - a[1] * b[0];
                let o = pt(B.x0 + 3.0, B.y0 + 2.0);
                let uv = |p: vectorcraft_geom::Point| (((p.x - o.x) * b[1] - (p.y - o.y) * b[0]) / d, (a[0] * (p.y - o.y) - a[1] * (p.x - o.x)) / d);
                let (mut cu, mut cv) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
                for i in &out.instances {
                    for c in [pt(B.x0, B.y0), pt(B.x1, B.y0), pt(B.x1, B.y1), pt(B.x0, B.y1)] {
                        let (u, v) = uv(i.transform * c);
                        cu = (cu.0.min(u), cu.1.max(u));
                        cv = (cv.0.min(v), cv.1.max(v));
                    }
                }
                let (mut gu, mut gv) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
                assert!(!out.guides.is_empty());
                for gd in &out.guides {
                    let crate::Guide::Line { from, to } = gd else { panic!("lines only") };
                    for p in [*from, *to] {
                        let (u, v) = uv(p);
                        // Every guide point is a whole lattice coordinate on one axis.
                        assert!((u - u.round()).abs() < 1e-6 || (v - v.round()).abs() < 1e-6, "{g:?}");
                        gu = (gu.0.min(u), gu.1.max(u));
                        gv = (gv.0.min(v), gv.1.max(v));
                    }
                }
                let e = 1e-6;
                assert!(gu.0 <= cu.0 + e && gu.1 >= cu.1 - e && gv.0 <= cv.0 + e && gv.1 >= cv.1 - e, "{g:?} guides enclose the copies");
                assert!(
                    gu.0 > cu.0 - 1.0 - e && gu.1 < cu.1 + 1.0 + e && gv.0 > cv.0 - 1.0 - e && gv.1 < cv.1 + 1.0 + e,
                    "{g:?} guides stop at the copies"
                );
            }
        }
    }

    #[test]
    fn a_square_cell_sized_p1_motif_is_tiled_by_its_guides() {
        // The layer is 30 x 35; a 30 x 35 cell: the guides are exactly the cell borders.
        let out = wp(WallpaperGroup::P1, json!({"cols": 3, "rows": 2, "cellWidth": 30, "cellHeight": 35, "angle": 90}));
        assert_eq!(out.guides.len(), 4 + 3);
        let ends: Vec<_> = out
            .guides
            .iter()
            .filter_map(|g| match g {
                crate::Guide::Line { from, to } => Some((*from, *to)),
                _ => None,
            })
            .collect();
        assert!(ends.iter().any(|(f, t)| near(*f, pt(10.0, 20.0)) && near(*t, pt(100.0, 20.0))));
        assert!(ends.iter().any(|(f, t)| near(*f, pt(10.0, 20.0)) && near(*t, pt(10.0, 90.0))));
    }

    #[test]
    fn changing_the_group_keeps_the_other_params() {
        let mut m = crate::Modifier::new("m1", "wallpaper").unwrap();
        m.set_props(json!({"cols": 5, "rows": 4, "cellWidth": 60, "cellHeight": 45, "angle": 70, "originX": 3, "originY": 4}).as_object().unwrap())
            .unwrap();
        for g in WallpaperGroup::ALL {
            m.set_props(json!({"group": g}).as_object().unwrap()).unwrap();
            let crate::ModifierKind::Wallpaper(p) = &m.kind else { panic!("wallpaper") };
            assert_eq!((p.group, p.cols, p.rows), (g, 5, 4));
            assert_eq!((p.cell_width, p.cell_height, p.angle, p.origin_x, p.origin_y), (60.0, 45.0, 70.0, 3.0, 4.0));
        }
        assert!(m.set_props(json!({"group": "p5"}).as_object().unwrap()).is_err());
    }

    #[test]
    fn evaluator_clamps_match_the_documented_ranges() {
        for g in WallpaperGroup::ALL {
            let out = wp(g, json!({"cellWidth": 0, "cellHeight": 0, "angle": 0}));
            assert!(out.instances.iter().all(|i| i.transform.is_finite()), "{g:?}");
            assert!(!out.warnings.is_empty(), "{g:?}: out-of-range values warn");
            let out = wp(g, json!({"cols": 0, "rows": 0}));
            assert_eq!(out.instances.len(), group_ops(g, 100.0, 70.0, 75.0).len(), "{g:?}: 0 cells reads as 1");
        }
    }
}
