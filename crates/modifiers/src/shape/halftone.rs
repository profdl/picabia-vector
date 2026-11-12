//! The `screen` geometry row (PIC20.5c): each path becomes the dots of a vector halftone.
//!
//! Written for Picabia (tlprocedural has no Screen). For each path the spot-function field of
//! [`crate::screen`] is sampled over the path's bounds and contoured by Picabia's own marching
//! squares ([`crate::contour`]); the rings are fitted with curves (Simplify's least-squares fit,
//! within a fraction of the sample step), then clipped to the path's outline with Pathfinder's
//! intersect (`vectorcraft_pathops`). The dots have no stroke. Their ink (the lead's decision,
//! PIC20.5d): with the tone read from the path's paint (`fill`) they are **black**, the paint giving
//! the tone only (inking them in the paint they read would darken twice); with a fixed or an image
//! tone they take the path's fill (its stroke's paint when it has no fill; black with neither),
//! except a near-white one ([`NEAR_WHITE`], PIC20.5e), which inks them black.
//!
//! # Clipping only what crosses the edge
//! A dot wholly inside the outline is kept as it is and one wholly outside is dropped; only the
//! dots that cross the outline's edge (or enclose the whole outline) go through the boolean, so a
//! large screen costs one small intersect. The outline is flattened into straight pieces kept in
//! horizontal bands, for the crossing test and an inside test. The boolean's outer contours are
//! turned clockwise (on screen) like the traced ones, and the result is filled non-zero: a hole kept
//! as it is inside a clipped dot still cuts it, and two fitted dots that touch still read as ink.
//!
//! # Page-fixed
//! The screen is laid on the page, not on the object: its cells sit at whole multiples of the
//! period from the page origin (turned by the angle), so copies below a copy row, each screened
//! where it sits, line up as one screen; an image's tone is read where the image is.
//!
//! # Caps
//! The sampled grid stays within [`crate::contour::MAX_CELLS`] (a finer step is coarsened, with a
//! note). Before tracing, the anchors are estimated from the screen's cells over the path
//! ([`SPOT_ANCHORS`] each) and checked against the copy's share of the geometry cap; after fitting
//! and clipping, the real count is checked again. Over either, the path is left as it is, with a
//! note. An outline of more than [`MAX_CLIP_SEGMENTS`] segments or [`MAX_OUTLINE_PIECES`] flattened
//! pieces is not screened; its pieces fill at most [`MAX_BAND_ENTRIES`] band slots (fewer, taller bands when they
//! would fill more), and the clip's tests are counted before they run: past [`MAX_CLIP_WORK`] piece
//! visits, or when the edge dots touch more than [`MAX_EDGE_CONTACTS`] pieces (each a crossing the
//! boolean must cut), the path is not screened either. Each with a note.

use std::sync::Arc;

use vectorcraft_doc::color::gradient::Ramp;
use vectorcraft_doc::color::{Color, GradientGeom, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, FillLayer, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, PathEl, Point, Rect, SubPath};

use crate::contour::Ring;
use crate::model::{Screen, SpotShape, ToneSource};
use crate::target::ToneTarget;

/// The anchors one screen cell is expected to hold once fitted (a round dot fits in about four
/// curves): the estimate checked against the cap before tracing.
pub const SPOT_ANCHORS: usize = 4;
/// The most anchors an outline may have, and the most straight pieces it is flattened into for the
/// inside and crossing tests.
pub const MAX_OUTLINE_PIECES: usize = 200_000;
/// The most segments an outline may have: the dots on its edge are cut by a boolean with it.
pub const MAX_CLIP_SEGMENTS: usize = 20_000;
/// The most outline pieces the dots on the edge may touch, all together (each touch may be a
/// crossing the boolean must find and cut): past it the path isn't screened.
pub const MAX_EDGE_CONTACTS: usize = 20_000;
/// The most band slots those pieces fill (a piece fills one in every band it spans).
pub const MAX_BAND_ENTRIES: usize = 2_000_000;
/// The most piece visits the clip's crossing and inside tests may make for one path.
pub const MAX_CLIP_WORK: u64 = 50_000_000;
/// The most horizontal bands those pieces are sorted into.
const MAX_BANDS: usize = 4096;
/// The fit's tolerance, as a share of the sample step.
const FIT_TOLERANCE: f64 = 0.15;
/// The outline's flattening tolerance, as a share of the sample step.
const FLATTEN_TOLERANCE: f64 = 0.05;

/// A screen row ready to run: its params clamped, and its image resolved when its tone comes from
/// one (`None` for the other tones; `Err` says why the image can't be read, and the row is skipped).
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenRow {
    pub params: Screen,
    pub image: Option<Result<Arc<ToneTarget>, String>>,
}

/// What screening paths needs and says: the anchors left in the copy's share, the notes.
pub(super) struct Run<'a> {
    pub room: usize,
    pub notes: &'a mut Vec<String>,
    pub skipped: bool,
}

impl Run<'_> {
    fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }
}

/// Screen every path in `n` (see the module docs). Returns `false` when the row can't run at all
/// (its image can't be read): `n` is left as it is.
pub(super) fn screen_node(n: &mut Node, row: &ScreenRow, run: &mut Run<'_>) -> bool {
    let image = match (&row.params.tone, &row.image) {
        (ToneSource::Image, Some(Ok(t))) => Some(t.clone()),
        (ToneSource::Image, Some(Err(e))) => {
            run.note(format!("Screen: {e}"));
            return false;
        }
        (ToneSource::Image, None) => {
            run.note("Screen: choose the image it reads its tone from; it is skipped".into());
            return false;
        }
        _ => None,
    };
    walk(n, row, image.as_deref(), run, 0);
    true
}

fn walk(n: &mut Node, row: &ScreenRow, image: Option<&ToneTarget>, run: &mut Run<'_>, depth: usize) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    let appearance = n.appearance.clone();
    match &mut n.kind {
        // Guides (path and ruler guides) don't print, and a clipping path's own paint isn't drawn.
        NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { path, rule, live, .. } => {
            let paint = ink(&appearance);
            // The outline's box, before the dots replace it: a gradient stays where it was.
            let bounds = path.bounds();
            if let Some(dots) = screen_outline(path, *rule, &paint, row, image, run) {
                *path = dots;
                *rule = FillRule::NonZero;
                *live = None;
                paint_dots(n, dot_ink(row, paint), bounds);
            }
        }
        NodeKind::Compound { children, rule } => {
            // The members paint as one with the compound's appearance: one outline.
            let outline = PathData::new(
                children
                    .iter()
                    .filter_map(|c| match &c.kind {
                        NodeKind::Path { path, .. } => Some(path.subpaths.iter().cloned()),
                        _ => None,
                    })
                    .flatten()
                    .collect(),
            );
            let paint = ink(&appearance);
            let Some(first) = children.first().cloned() else { return };
            if !matches!(first.kind, NodeKind::Path { .. }) {
                return;
            }
            let bounds = outline.bounds();
            if let Some(dots) = screen_outline(&outline, *rule, &paint, row, image, run) {
                let mut member = Arc::unwrap_or_clone(first);
                if let NodeKind::Path { path, rule: r, live, .. } = &mut member.kind {
                    *path = dots;
                    *r = FillRule::NonZero;
                    *live = None;
                }
                *children = vec![Arc::new(member)];
                *rule = FillRule::NonZero;
                paint_dots(n, dot_ink(row, paint), bounds);
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children.iter_mut() {
                walk(Arc::make_mut(c), row, image, run, depth + 1);
            }
        }
        _ => run.skipped = true,
    }
}

/// What the dots are inked with: black when their tone came from `paint` (it gives the tone only),
/// `paint` itself for a fixed or an image tone, unless it is near-white (PIC20.5e: white dots on
/// white paper wouldn't show, as on a new rectangle's default white fill): then black.
fn dot_ink(row: &ScreenRow, paint: Paint) -> Paint {
    match row.params.tone {
        ToneSource::Fill => Paint::solid(Color::BLACK),
        ToneSource::Fixed | ToneSource::Image if near_white(&paint) => Paint::solid(Color::BLACK),
        ToneSource::Fixed | ToneSource::Image => paint,
    }
}

/// The luminance (Rec. 709) above which an ink counts as near-white.
pub const NEAR_WHITE: f64 = 0.9;

/// Whether `paint` would ink near-white dots: a solid colour above [`NEAR_WHITE`] luminance, a
/// gradient whose every stop is; never a pattern.
fn near_white(paint: &Paint) -> bool {
    let light = |c: &Color| 1.0 - darkness(c.to_rgb(), 1.0) > NEAR_WHITE;
    match paint {
        Paint::None => true,
        Paint::Solid { color, .. } => light(color),
        Paint::Gradient(g) => g.gradient.stops.iter().all(|s| light(&s.color)),
        Paint::Pattern { .. } => false,
    }
}

/// The path's paint: the fill, else the stroke's paint, else black (the tone `fill` reads, and the
/// ink of a fixed or an image tone).
fn ink(a: &vectorcraft_doc::Appearance) -> Paint {
    let fill = a.fill_paint();
    if !fill.is_none() {
        return fill;
    }
    let stroke = a.stroke_paint();
    if !stroke.is_none() {
        return stroke;
    }
    Paint::solid(Color::BLACK)
}

/// `n` painted with `paint` alone (no stroke; its effects kept). A gradient that was fitted to the
/// outline's box stays where it was (the dots' box is a little smaller).
fn paint_dots(n: &mut Node, mut paint: Paint, bounds: Option<Rect>) {
    if let (Paint::Gradient(g), Some(b)) = (&mut paint, bounds) {
        g.pin(b);
    }
    let a = &mut *n.appearance;
    a.items = vec![AppearanceItem::Fill(FillLayer::new(paint))];
    a.contents_index = None;
}

/// A paint's darkness where each point sits.
enum PaintTone {
    Flat(f64),
    Ramp { geom: GradientGeom, kind: GradientKind, ramp: Ramp },
}

impl PaintTone {
    fn of(paint: &Paint, bounds: Rect, run: &mut Run<'_>) -> PaintTone {
        match paint {
            Paint::None => PaintTone::Flat(1.0),
            Paint::Solid { color, .. } => PaintTone::Flat(darkness(color.to_rgb(), 1.0)),
            Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => {
                run.note("Screen reads a freeform gradient as the average of its colours".into());
                let stops = &g.gradient.stops;
                let sum: f64 = stops.iter().map(|s| darkness(s.color.to_rgb(), s.opacity)).sum();
                PaintTone::Flat(if stops.is_empty() { 1.0 } else { sum / stops.len() as f64 })
            }
            Paint::Gradient(g) => {
                PaintTone::Ramp { geom: g.resolve(bounds), kind: g.gradient.kind, ramp: Ramp::new(&g.gradient, &|c: &Color| c.to_rgb()) }
            }
            Paint::Pattern { .. } => {
                run.note("Screen doesn't read a pattern's darkness; it screens it at 50%".into());
                PaintTone::Flat(0.5)
            }
        }
    }

    fn at(&self, p: Point) -> f64 {
        match self {
            PaintTone::Flat(t) => *t,
            PaintTone::Ramp { geom, kind, ramp } => {
                let t = geom.param_at(*kind, p);
                let [r, g, b, a] = ramp.at(if t.is_finite() { t as f32 } else { 0.0 });
                darkness([r, g, b], a)
            }
        }
    }
}

/// How much ink a colour is: one less its Rec. 709 luminance, times its opacity.
fn darkness([r, g, b]: [f32; 3], alpha: f32) -> f64 {
    let lum = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
    let d = ((1.0 - lum) * f64::from(alpha)).clamp(0.0, 1.0);
    if d.is_finite() { d } else { 0.0 }
}

/// The dots of `row` over `path` (filled by `rule`), clipped to it; `None` leaves the path as it
/// is (empty, over the cap, or the trace refused), with a note.
fn screen_outline(
    path: &PathData,
    rule: FillRule,
    paint: &Paint,
    row: &ScreenRow,
    image: Option<&ToneTarget>,
    run: &mut Run<'_>,
) -> Option<PathData> {
    let bounds = path.bounds().filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.width() > 0.0 && b.height() > 0.0)?;
    // The edge dots are cut by a boolean with the whole outline: its cost grows with the outline.
    let segments = path.subpaths.iter().fold(0usize, |n, sp| n.saturating_add(sp.segment_count()));
    if segments > MAX_CLIP_SEGMENTS {
        run.note(format!(
            "Screen: an outline of more than {MAX_CLIP_SEGMENTS} segments isn't screened (simplify it, or split it into several paths)"
        ));
        return None;
    }
    let p = &row.params;
    let screen = crate::screen::Screen::new(p.spot, p.lpi, p.angle, p.quality);
    // The anchors to expect, from the screen's cells over the path's box, before anything is made.
    let period = screen.period();
    let cells = ((bounds.width() / period).ceil() + 1.0) * ((bounds.height() / period).ceil() + 1.0);
    let expected = cells * SPOT_ANCHORS as f64;
    if !expected.is_finite() || expected > run.room as f64 {
        let total = super::MAX_GEOMETRY_ANCHORS;
        run.note(format!("Screen would make more than {total} anchors for this object and its copies (about {} cells); raise lpi's period or shrink the art, it is skipped", cells as u64));
        return None;
    }
    let paint_tone = match p.tone {
        ToneSource::Fill => Some(PaintTone::of(paint, bounds, run)),
        _ => None,
    };
    let fixed = (p.amount / 100.0).clamp(0.0, 1.0);
    let invert = p.invert;
    let tone = |q: Point| -> f64 {
        let t = match (p.tone, &paint_tone, image) {
            (ToneSource::Fixed, ..) => fixed,
            (ToneSource::Image, _, Some(img)) => img.darkness_at(q),
            (_, Some(pt), _) => pt.at(q),
            _ => 0.0,
        };
        if invert { 1.0 - t } else { t }
    };
    // The samples sit on a lattice fixed to the page too (the box widened to whole steps from the
    // page origin), so copies a whole number of steps apart trace the same dots.
    let bounds = match screen.step_for(bounds) {
        Some((step, _)) => {
            let snap = |v: f64| (v / step).floor() * step;
            let r = Rect::new(snap(bounds.x0), snap(bounds.y0), bounds.x1, bounds.y1);
            if [r.x0, r.y0].iter().all(|v| v.is_finite()) { r } else { bounds }
        }
        None => bounds,
    };
    let traced = match screen.trace(bounds, tone) {
        Ok(t) => t,
        Err(e) => {
            run.note(format!("Screen: {e}; it is skipped"));
            return None;
        }
    };
    if traced.coarsened {
        run.note(format!("Screen traces this art more coarsely than its quality asks, to stay within {} samples", crate::contour::MAX_CELLS));
    }
    let dots = fit(&traced.rings, traced.step, p.spot);
    let clipped = clip(&dots, path, rule, traced.step, run)?;
    let anchors = clipped.anchor_count();
    if anchors > run.room {
        let total = super::MAX_GEOMETRY_ANCHORS;
        run.note(format!("Screen would make more than {total} anchors for this object and its copies; it is skipped"));
        return None;
    }
    run.room -= anchors;
    Some(clipped)
}

/// The rings fitted with curves (Simplify, within [`FIT_TOLERANCE`] of the step). Round and
/// elliptical dots are fitted smooth through their polygon's turns; the other spots keep their
/// corners.
fn fit(rings: &[Ring], step: f64, spot: SpotShape) -> PathData {
    let corner_angle_deg = if matches!(spot, SpotShape::Round | SpotShape::Ellipse) { 70.0 } else { 35.0 };
    let opts = vectorcraft_pathops::SimplifyOptions { tolerance: step * FIT_TOLERANCE, corner_angle_deg, ..Default::default() };
    vectorcraft_pathops::simplify_with(&PathData::new(rings.iter().map(Ring::to_subpath).collect()), &opts)
}

/// `dots` clipped to `outline` (filled by `rule`): the dots inside kept, those outside dropped, those
/// on the edge intersected with it (see the module docs). `None` (with a note) when the outline is
/// too detailed to clip within the caps.
fn clip(dots: &PathData, outline: &PathData, rule: FillRule, step: f64, run: &mut Run<'_>) -> Option<PathData> {
    let Some(edge) = Edge::new(outline, rule, step * FLATTEN_TOLERANCE) else {
        run.note(format!("Screen: the outline flattens into more than {MAX_OUTLINE_PIECES} pieces; it isn't screened"));
        return None;
    };
    let hulls: Vec<Option<Rect>> = dots.subpaths.iter().map(hull).collect();
    // The tests' cost, counted before they run.
    let work = hulls.iter().flatten().filter(|b| overlaps(**b, edge.bbox)).fold(0u64, |w, b| w.saturating_add(edge.cost(*b)));
    if work > MAX_CLIP_WORK {
        run.note(
            "Screen: the outline is too detailed to clip this many dots to; it isn't screened (simplify the outline or lower the lines per inch)"
                .into(),
        );
        return None;
    }
    let mut kept: Vec<SubPath> = Vec::new();
    let mut crossing: Vec<SubPath> = Vec::new();
    let mut contacts = 0usize;
    for (sp, b) in dots.subpaths.iter().zip(&hulls) {
        let Some(b) = *b else { continue };
        if !overlaps(b, edge.bbox) {
            continue;
        }
        let encloses = b.x0 <= edge.bbox.x0 && b.y0 <= edge.bbox.y0 && b.x1 >= edge.bbox.x1 && b.y1 >= edge.bbox.y1;
        if encloses || edge.crosses(b) {
            contacts = contacts.saturating_add(edge.touching(b));
            if contacts > MAX_EDGE_CONTACTS {
                run.note("Screen: the dots would cross the outline's edge too often to cut (a very detailed edge); it isn't screened (simplify the outline or lower the lines per inch)".into());
                return None;
            }
            crossing.push(sp.clone());
        } else if sp.anchors.first().is_some_and(|a| edge.inside(a.p)) {
            kept.push(sp.clone());
        }
    }
    if !crossing.is_empty() {
        let edge_dots = PathData::new(crossing);
        match vectorcraft_pathops::try_boolean(
            &edge_dots,
            FillRule::NonZero,
            outline,
            rule,
            vectorcraft_pathops::BoolOp::Intersect,
            vectorcraft_pathops::DEFAULT_PRECISION,
        ) {
            Ok(cut) => kept.extend(clockwise(cut).subpaths),
            Err(e) => run.note(format!("Screen couldn't cut the dots at the outline's edge ({e}); those dots are left out")),
        }
    }
    Some(PathData::new(kept))
}

/// `p` with its outer contours clockwise on screen, as the traced dots are: the boolean's contours
/// are oriented alike (outers one way, holes the other), so when its largest contour (an outer)
/// runs counter-clockwise, every contour is reversed.
fn clockwise(p: PathData) -> PathData {
    let largest = p.subpaths.iter().map(SubPath::signed_area).filter(|a| a.is_finite()).max_by(|a, b| a.abs().total_cmp(&b.abs()));
    match largest {
        Some(a) if a < 0.0 => PathData::new(
            p.subpaths
                .iter()
                .map(|sp| {
                    let mut r = sp.clone();
                    r.reverse();
                    r
                })
                .collect(),
        ),
        _ => p,
    }
}

/// The box of a subpath's anchors and handles (it holds the curve), `None` when empty or not finite.
fn hull(sp: &SubPath) -> Option<Rect> {
    let mut pts = sp.anchors.iter().flat_map(|a| [a.p, a.h_in, a.h_out]);
    let first = pts.next()?;
    let r = pts.fold(Rect::from_points(first, first), |r, q| r.union_pt(q));
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// An outline flattened into straight pieces, sorted into horizontal bands.
struct Edge {
    pieces: Vec<(Point, Point)>,
    bands: Vec<Vec<u32>>,
    /// `prefix[k]`: the slots in the bands before band `k` (for counting a test's cost).
    prefix: Vec<u64>,
    y0: f64,
    band_h: f64,
    bbox: Rect,
    rule: FillRule,
    /// How far the pieces may stray from the curve.
    tol: f64,
}

impl Edge {
    fn new(outline: &PathData, rule: FillRule, tol: f64) -> Option<Edge> {
        let tol = if tol.is_finite() && tol > 0.0 { tol } else { 0.01 };
        let mut pieces: Vec<(Point, Point)> = Vec::new();
        let (mut start, mut last) = (Point::ZERO, Point::ZERO);
        let mut over = false;
        let mut push = |pieces: &mut Vec<(Point, Point)>, a: Point, b: Point| {
            if pieces.len() >= MAX_OUTLINE_PIECES {
                over = true;
            } else if a != b {
                pieces.push((a, b));
            }
        };
        let bez = outline.to_bezpath();
        vectorcraft_geom::kurbo::flatten(bez.elements().iter().copied(), tol, |el| match el {
            PathEl::MoveTo(p) => {
                // Filling closes every subpath.
                push(&mut pieces, last, start);
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                push(&mut pieces, last, p);
                last = p;
            }
            PathEl::ClosePath => {
                push(&mut pieces, last, start);
                last = start;
            }
            _ => {}
        });
        push(&mut pieces, last, start);
        if over {
            return None;
        }
        let Some(&(first, _)) = pieces.first() else { return Some(Edge::empty(rule, tol)) };
        let bbox = pieces.iter().fold(Rect::from_points(first, first), |r, (a, b)| r.union_pt(*a).union_pt(*b));
        if ![bbox.x0, bbox.y0, bbox.x1, bbox.y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        // As many bands as the square root of the pieces, fewer (taller) while the pieces would fill
        // more than MAX_BAND_ENTRIES slots: counted before anything is allocated; one band holds
        // each piece once.
        let mut count = ((pieces.len() as f64).sqrt().ceil() as usize).clamp(1, MAX_BANDS);
        let slots = |count: usize| -> usize {
            let band_h = (bbox.height() / count as f64).max(1e-9);
            let band = |y: f64| band_of(y, bbox.y0, band_h, count);
            pieces.iter().fold(0usize, |n, (a, b)| n.saturating_add(band(a.y.max(b.y)) - band(a.y.min(b.y)) + 1))
        };
        while count > 1 && slots(count) > MAX_BAND_ENTRIES {
            count /= 2;
        }
        let band_h = (bbox.height() / count as f64).max(1e-9);
        let mut e = Edge { pieces: Vec::new(), bands: vec![Vec::new(); count], prefix: Vec::new(), y0: bbox.y0, band_h, bbox, rule, tol };
        for (k, (a, b)) in pieces.iter().enumerate() {
            let (lo, hi) = (e.band(a.y.min(b.y)), e.band(a.y.max(b.y)));
            for band in e.bands.iter_mut().take(hi + 1).skip(lo) {
                band.push(u32::try_from(k).unwrap_or(u32::MAX));
            }
        }
        e.pieces = pieces;
        e.prefix = std::iter::once(0u64)
            .chain(e.bands.iter().scan(0u64, |sum, b| {
                *sum = sum.saturating_add(b.len() as u64);
                Some(*sum)
            }))
            .collect();
        Some(e)
    }

    fn empty(rule: FillRule, tol: f64) -> Edge {
        Edge { pieces: Vec::new(), bands: Vec::new(), prefix: vec![0], y0: 0.0, band_h: 1.0, bbox: Rect::ZERO, rule, tol }
    }

    fn band(&self, y: f64) -> usize {
        band_of(y, self.y0, self.band_h, self.bands.len())
    }

    /// The piece visits the crossing test of box `r` makes (the inside test makes fewer).
    fn cost(&self, r: Rect) -> u64 {
        let r = self.widened(r);
        let (lo, hi) = (self.band(r.y0), self.band(r.y1));
        match (self.prefix.get(lo), self.prefix.get(hi + 1)) {
            (Some(a), Some(b)) => b.saturating_sub(*a).saturating_add(1),
            _ => 1,
        }
    }

    fn widened(&self, r: Rect) -> Rect {
        Rect::new(r.x0 - self.tol * 2.0, r.y0 - self.tol * 2.0, r.x1 + self.tol * 2.0, r.y1 + self.tol * 2.0)
    }

    fn pieces_in(&self, band: usize) -> impl Iterator<Item = &(Point, Point)> + '_ {
        self.bands.get(band).into_iter().flatten().filter_map(|&k| self.pieces.get(k as usize))
    }

    /// Whether `p` is inside the outline (its fill rule), by a ray to the right.
    fn inside(&self, p: Point) -> bool {
        if !(p.x.is_finite() && p.y.is_finite()) || p.y < self.bbox.y0 || p.y > self.bbox.y1 {
            return false;
        }
        let mut winding = 0i64;
        for (a, b) in self.pieces_in(self.band(p.y)) {
            let up = a.y <= p.y && p.y < b.y;
            let down = b.y <= p.y && p.y < a.y;
            if !(up || down) {
                continue;
            }
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if x > p.x {
                winding += if up { 1 } else { -1 };
            }
        }
        match self.rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }

    /// How many pieces' boxes meet box `r` (widened as [`Edge::crosses`] widens it; a piece over
    /// several bands counted in each).
    fn touching(&self, r: Rect) -> usize {
        let r = self.widened(r);
        (self.band(r.y0)..=self.band(r.y1)).map(|band| self.pieces_in(band).filter(|(a, b)| overlaps(r, Rect::from_points(*a, *b))).count()).sum()
    }

    /// Whether a piece of the outline may pass through box `r` (the pieces' boxes, widened by the
    /// flattening tolerance).
    fn crosses(&self, r: Rect) -> bool {
        let r = self.widened(r);
        if !overlaps(r, self.bbox) {
            return false;
        }
        (self.band(r.y0)..=self.band(r.y1)).any(|band| self.pieces_in(band).any(|(a, b)| overlaps(r, Rect::from_points(*a, *b))))
    }
}

/// The band of `count` (each `band_h` tall from `y0`) holding `y`, clamped to the bands.
fn band_of(y: f64, y0: f64, band_h: f64, count: usize) -> usize {
    let last = count.max(1) - 1;
    let k = ((y - y0) / band_h).floor();
    if k.is_finite() { (k.max(0.0) as usize).min(last) } else { 0 }
}

/// The band slots `outline`'s pieces fill (`None` past [`MAX_OUTLINE_PIECES`] pieces), for tests.
#[cfg(test)]
pub(super) fn band_slots(outline: &PathData, rule: FillRule, tol: f64) -> Option<usize> {
    Edge::new(outline, rule, tol).map(|e| e.bands.iter().map(Vec::len).sum())
}
