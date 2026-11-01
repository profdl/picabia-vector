//! Limits and checks (stage 8), through [`compute`]: one synthetic shape per problem kind, fix vs
//! flag mode, the list cap.

use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilLimitMode, StencilTieMode};

use crate::{MAX_PROBLEMS, Paint, ProblemKind, StencilOut, compute};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// 1 mm per unit, a 10 mm margin.
const B: Rect = Rect::new(0.0, 0.0, 100.0, 100.0);

fn paint(path: BezPath, colour: [f32; 4], order: u32) -> Paint {
    Paint { path, colour, order }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.01)
}

/// A black ground with `white` shapes on top, then `black` shapes over them.
fn design(white: &[BezPath], black: &[BezPath]) -> Vec<Paint> {
    let mut v = vec![paint(B.to_path(0.01), BLACK, 0)];
    v.extend(white.iter().map(|s| paint(s.clone(), WHITE, 1)));
    v.extend(black.iter().map(|s| paint(s.clone(), BLACK, 2)));
    v
}

/// Default limits (0.3 mm cut, 0.5 mm bridge, 0.5 mm², 30°), gauze, no registration holes.
fn params(mode: StencilLimitMode) -> Stencil {
    Stencil { width_mm: 100.0, margin_mm: 10.0, tie_mode: StencilTieMode::Gauze, hoshi: false, limit_mode: mode, ..Stencil::default() }
}

fn run(paints: &[Paint], p: &Stencil) -> StencilOut {
    compute(paints, p, B).unwrap()
}

fn of(out: &StencilOut, kind: ProblemKind) -> Vec<Rect> {
    out.problems.iter().filter(|p| p.kind == kind).map(|p| p.bounds).collect()
}

/// The counts and the list agree (below the cap), and only `kinds` occur.
fn only(out: &StencilOut, kinds: &[ProblemKind]) {
    for k in ProblemKind::ALL {
        let n = out.stats.problems_by_kind.get(k);
        assert_eq!(n, of(out, k).len(), "{k:?}");
        assert_eq!(n > 0, kinds.contains(&k), "{k:?}: {:?}", out.problems);
    }
}

fn near(r: Rect, p: Point, tol: f64) -> bool {
    r.inflate(tol, tol).contains(p)
}

#[test]
fn a_clean_design_has_no_problems() {
    let paints = design(&[rect(20.0, 20.0, 40.0, 50.0), rect(60.0, 20.0, 80.0, 80.0)], &[]);
    for mode in [StencilLimitMode::Fix, StencilLimitMode::Flag] {
        let out = run(&paints, &params(mode));
        only(&out, &[]);
        assert!(out.problems.is_empty() && out.warnings.is_empty(), "{mode:?}: {:?} {:?}", out.problems, out.warnings);
    }
}

#[test]
fn a_thin_paper_bridge() {
    // Two squares 0.3 mm apart: the paper between them is narrower than the 0.5 mm bridge.
    let paints = design(&[rect(10.0, 10.0, 20.0, 20.0), rect(20.3, 10.0, 30.3, 20.0)], &[]);
    let flag = run(&paints, &params(StencilLimitMode::Flag));
    only(&flag, &[ProblemKind::NarrowPaper]);
    let b = of(&flag, ProblemKind::NarrowPaper)[0];
    assert!(b.x0 >= 19.9 && b.x1 <= 20.4 && b.height() > 8.0, "{b:?}");
    // Fix mode cuts it through: nothing left to flag.
    let fix = run(&paints, &params(StencilLimitMode::Fix));
    only(&fix, &[]);
    assert_eq!(fix.stats.cut_pieces, 1);
    // A thin margin is narrow paper too.
    let edge = run(&design(&[rect(0.2, 40.0, 30.0, 60.0)], &[]), &Stencil { margin_mm: 0.0, ..params(StencilLimitMode::Flag) });
    only(&edge, &[ProblemKind::NarrowPaper]);
}

#[test]
fn a_thin_cut() {
    // A 0.2 mm slit (6 mm², above the area limit).
    let paints = design(&[rect(10.0, 10.0, 10.2, 40.0), rect(50.0, 50.0, 60.0, 60.0)], &[]);
    let flag = run(&paints, &params(StencilLimitMode::Flag));
    only(&flag, &[ProblemKind::NarrowCut]);
    let b = of(&flag, ProblemKind::NarrowCut)[0];
    assert!(b.x0 >= 9.9 && b.x1 <= 10.3 && b.height() > 25.0, "{b:?}");
    assert_eq!(flag.stats.cut_pieces, 2, "flag mode keeps it");
    only(&run(&paints, &params(StencilLimitMode::Fix)), &[]);
}

#[test]
fn a_sharp_paper_corner() {
    // A paper island: a triangle with a 14° tip at (70, 50), its other corners about 83°.
    let tri = {
        let mut p = BezPath::new();
        p.move_to((30.0, 45.0));
        p.line_to((70.0, 50.0));
        p.line_to((30.0, 55.0));
        p.close_path();
        p
    };
    let paints = design(&[rect(20.0, 20.0, 80.0, 80.0)], &[tri]);
    let p = Stencil { min_cut_width_mm: 0.0, min_bridge_mm: 0.0, ..params(StencilLimitMode::Flag) };
    let out = run(&paints, &p);
    only(&out, &[ProblemKind::SharpCorner, ProblemKind::Untied]);
    let corners = of(&out, ProblemKind::SharpCorner);
    assert_eq!(corners.len(), 1, "{corners:?}");
    assert!(near(corners[0], Point::new(70.0, 50.0), 0.5), "{corners:?}");
    // A smaller limit lets it be; a larger one catches the base corners too.
    only(&run(&paints, &Stencil { min_corner_angle: 10.0, ..p.clone() }), &[ProblemKind::Untied]);
    assert_eq!(run(&paints, &Stencil { min_corner_angle: 90.0, ..p.clone() }).stats.problems_by_kind.get(ProblemKind::SharpCorner), 3);
    // The cut's own corners (a square, convex) and the paper's reflex corners are never sharp.
    only(&run(&design(&[rect(20.0, 20.0, 80.0, 80.0)], &[]), &Stencil { min_corner_angle: 179.0, ..p }), &[]);
}

#[test]
fn a_tiny_hole_and_a_tiny_cut() {
    // A 0.6 × 0.6 mm paper dot in a cut-out (0.36 mm² < 0.5) and a 0.6 mm speck of cut.
    let paints = design(&[rect(20.0, 20.0, 40.0, 40.0), rect(60.0, 60.0, 60.6, 60.6)], &[rect(30.0, 30.0, 30.6, 30.6)]);
    let flag = run(&paints, &params(StencilLimitMode::Flag));
    only(&flag, &[ProblemKind::SmallHole, ProblemKind::SmallCut, ProblemKind::Untied]);
    assert!(near(of(&flag, ProblemKind::SmallHole)[0], Point::new(30.3, 30.3), 0.1));
    assert!(near(of(&flag, ProblemKind::SmallCut)[0], Point::new(60.3, 60.3), 0.1));
    assert_eq!(of(&flag, ProblemKind::Untied).len(), 1, "the dot floats (gauze)");
    // Fix mode drops the speck and fills the hole.
    let fix = run(&paints, &params(StencilLimitMode::Fix));
    only(&fix, &[]);
    assert_eq!((fix.stats.dropped_specks, fix.stats.filled_holes), (1, 1));
}

#[test]
fn untied_islands_are_problems() {
    let paints = design(&[rect(20.0, 20.0, 80.0, 80.0)], &[rect(30.0, 30.0, 70.0, 70.0)]);
    let gauze = run(&paints, &params(StencilLimitMode::Fix));
    only(&gauze, &[ProblemKind::Untied]);
    assert_eq!(gauze.stats.problems_by_kind.get(ProblemKind::Untied), gauze.stats.untied);
    assert!(near(of(&gauze, ProblemKind::Untied)[0], Point::new(50.0, 50.0), 0.0));
    let tied = run(&paints, &Stencil { tie_mode: StencilTieMode::Ties, ..params(StencilLimitMode::Fix) });
    only(&tied, &[]);
    // Ties thinner than the bridge are narrow paper, found near them in fix mode too.
    let thin = run(&paints, &Stencil { tie_mode: StencilTieMode::Ties, tie_width_mm: 0.3, ..params(StencilLimitMode::Fix) });
    only(&thin, &[ProblemKind::NarrowPaper]);
    assert_eq!(of(&thin, ProblemKind::NarrowPaper).len(), 2, "one per tie: {:?}", thin.problems);
}

#[test]
fn the_list_is_capped_and_shares_out_the_kinds() {
    // 900 small cuts (0.6 mm squares) and one thin slit.
    let mut white: Vec<BezPath> = (0..30)
        .flat_map(|i| (0..30).map(move |j| (f64::from(i) * 3.0 + 5.0, f64::from(j) * 3.0 + 5.0)))
        .map(|(x, y)| rect(x, y, x + 0.6, y + 0.6))
        .collect();
    white.push(rect(97.0, 5.0, 97.2, 60.0));
    let out = run(&design(&white, &[]), &Stencil { min_bridge_mm: 0.0, ..params(StencilLimitMode::Flag) });
    let c = out.stats.problems_by_kind;
    assert_eq!((c.get(ProblemKind::SmallCut), c.get(ProblemKind::NarrowCut)), (900, 1), "{c:?}");
    assert_eq!(c.total(), 901);
    assert_eq!(out.problems.len(), MAX_PROBLEMS);
    assert_eq!(of(&out, ProblemKind::NarrowCut).len(), 1, "every kind gets markers");
    assert!(out.problems.windows(2).all(|w| w[0].kind <= w[1].kind), "grouped by kind");
    let w = out.warnings.iter().find(|w| w.contains("aren't marked")).expect("a warning names the rest");
    assert!(w.contains("401 more") && w.contains("401 small cuts"), "{w}");
}

#[test]
fn kinds_have_ids() {
    let ids: Vec<&str> = ProblemKind::ALL.iter().map(|k| k.id()).collect();
    assert_eq!(ids, ["narrowPaper", "narrowCut", "smallCut", "smallHole", "sharpCorner", "untied"]);
}
