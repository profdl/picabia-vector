//! Gathering a stencil's design from a VectorCraft object: every visible fill and stroke under it
//! as a closed outline in document space, with its colour and paint order ([`Paint`]s, bottom
//! first), ready for [`compute`](crate::compute).
//!
//! What is read:
//! - groups and layers in paint order (their first child is the bottom one); a clip group's (or
//!   clipping layer's) first child clips the rest to its outline;
//! - paths and compound paths, by their fill rule, each fill and stroke of their appearance stack
//!   in its paint order (`items[0]` first). A stroke is outlined (its weight, caps, joins and
//!   mitre limit; Inside / Outside alignment by its closed path's fill) into a shape of its own;
//! - live objects (blends, envelopes, meshes, repeats, Intertwine, placed documents) as they
//!   expand; type through the caller's outliner (glyph outlines need the font engine, above this
//!   crate);
//! - objects with a Picabia modifier stack of their own, at each of their copies.
//!
//! What isn't, said once each in [`Gathered::warnings`]: images (trace them first), patterns,
//! symbol instances, effects; opacity masks (the masked art is cut whole); gradients cut as their
//! mean colour; dashes, width profiles, brushes and arrowheads are cut as plain strokes (weights
//! capped at [`MAX_STROKE_WIDTH`]). A clip group's clipping path cuts as the renderer paints it:
//! its fills behind the clipped art (clipped), its strokes over it (not clipped). Hidden objects, guides and
//! template layers are left out silently, as is an Image Trace object's hidden source image.
//!
//! An Image Trace result (a group of filled paths and compound paths) gathers as its paths, which
//! is the trace → stencil path.

use vectorcraft_color::{Color, Paint as Fill};
use vectorcraft_doc::appearance::{AppearanceItem, LineCap, LineJoin, StrokeAlign, StrokeLayer};
use vectorcraft_doc::live::{Outliner, expand_deep, is_live};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{Affine, BezPath};
use vectorcraft_geom::{FillRule, PathData};

use crate::ops::{BoolOp, boolean_step, simplify_with_rule};
use crate::{MAX_PAINTS, Paint};

/// Deepest nesting gathered below the stencil object.
pub const MAX_GATHER_DEPTH: usize = 64;
/// The most object visits (each copy counts once) one gather makes.
pub const MAX_GATHER_VISITS: usize = 200_000;
/// The widest stroke outlined, points (half of it is [`MAX_OFFSET_DISTANCE`](crate::MAX_OFFSET_DISTANCE));
/// wider weights are clamped.
pub const MAX_STROKE_WIDTH: f64 = 1.0e6;

/// The painted outlines of a stencil's design.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gathered {
    /// In document space, bottom first.
    pub paints: Vec<Paint>,
    /// Where each paint came from (same index as [`Gathered::paints`]).
    pub sources: Vec<Source>,
    /// What couldn't be cut as drawn (each said once).
    pub warnings: Vec<String>,
}

/// Where a gathered paint came from ([`Gathered::sources`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The object that paints it.
    pub node: NodeId,
    /// That object's name, when it has one.
    pub name: Option<String>,
    /// It is a stroke outlined into a shape of its own.
    pub is_stroke: bool,
    /// Its colour is a gradient's mean colour (not the paint as drawn).
    pub gradient: bool,
}

struct G<'a> {
    doc: &'a Document,
    text: Outliner<'a>,
    out: Gathered,
    visits: usize,
    full: bool,
}

/// The painted outlines of `node`'s design, placed by each of `copies` (maps of document space:
/// the copies the modifier rows above a stencil make; empty: the object as it is), bottom first.
/// The object's own modifier stack is not applied (it is the stencil's); stacks on objects inside
/// it are, with their copy-to-shape targets resolved in `doc` (the document `node` is in).
/// `text` outlines type (`None`: type is left out, with a warning).
pub fn gather<'a>(doc: &'a Document, node: &Node, copies: &[Affine], text: Outliner<'a>) -> Gathered {
    let mut g = G { doc, text, out: Gathered::default(), visits: 0, full: false };
    if copies.is_empty() {
        g.once(node, Affine::IDENTITY, None, 0);
    } else {
        for c in copies {
            g.once(node, *c, None, 0);
        }
    }
    g.out
}

/// [`gather`] for a design whose copies are reshaped by geometry rows above the stencil: `copies`
/// are the reshaped copies themselves, in document space (`vectorcraft_modifiers::shaped_copies`),
/// each gathered as it is.
pub fn gather_shaped<'a>(doc: &'a Document, copies: &[Node], text: Outliner<'a>) -> Gathered {
    let mut g = G { doc, text, out: Gathered::default(), visits: 0, full: false };
    for c in copies {
        g.once(c, Affine::IDENTITY, None, 0);
    }
    g.out
}

impl G<'_> {
    fn warn(&mut self, w: &str) {
        if !self.out.warnings.iter().any(|x| x == w) {
            self.out.warnings.push(w.to_string());
        }
    }

    /// Object `n` inside the stencil, placed by `xf`: each copy of a modifier stack on it.
    fn node(&mut self, n: &Node, xf: Affine, clip: Option<&BezPath>, depth: usize) {
        if !n.visible || n.opacity <= 0.0 || n.is_template() || self.full {
            return;
        }
        if n.extras().mask.is_some() {
            self.warn("opacity masks aren't cut; the masked art is cut whole");
        }
        match vectorcraft_modifiers::node_instances_in_doc(self.doc, n) {
            Some(out) if out.has_geometry() => {
                if out.stencil.is_some() {
                    self.warn("a stencil inside the stencil is cut from its design");
                }
                // Geometry rows reshape the copies: each is cut as drawn.
                let (copies, notes) = vectorcraft_modifiers::shaped_copies(n, &out);
                for note in &notes {
                    self.warn(note);
                }
                for c in &copies {
                    self.once(c, xf, clip, depth);
                }
            }
            Some(out) if !out.instances.is_empty() => {
                if out.stencil.is_some() {
                    self.warn("a stencil inside the stencil is cut from its design");
                }
                for inst in &out.instances {
                    self.once(n, xf * inst.transform, clip, depth);
                }
            }
            _ => self.once(n, xf, clip, depth),
        }
    }

    /// One placement of `n`: document space mapped by `xf`.
    fn once(&mut self, n: &Node, xf: Affine, clip: Option<&BezPath>, depth: usize) {
        self.visits += 1;
        if depth > MAX_GATHER_DEPTH || self.visits > MAX_GATHER_VISITS {
            self.warn("the design is nested too deep or repeats too often; part of it is left out");
            self.full = true;
            return;
        }
        if !xf.is_finite() || self.full {
            return;
        }
        if !n.appearance.effects.is_empty() || n.appearance.items.iter().any(|i| !i.effects().is_empty()) {
            self.warn("effects aren't cut");
        }
        if is_live(n) {
            let expanded = expand_deep(n, self.text);
            if is_live(&expanded) {
                self.warn("live art that doesn't expand isn't cut");
            } else {
                self.once(&expanded, xf, clip, depth + 1);
            }
            return;
        }
        match &n.kind {
            // Guides (path guides and ruler guides) draw on screen only.
            NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => {}
            NodeKind::Path { path, rule, .. } => {
                let bp = path.to_bezpath();
                self.appearance(n, &bp, *rule, path.is_closed(), xf, clip);
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                let closed = children.iter().filter_map(|c| c.path_data()).all(PathData::is_closed);
                self.appearance(n, &bp, *rule, closed, xf, clip);
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                self.group_paint(n);
                let Some((first, rest)) = children.split_first() else { return };
                let mut shapes = first.clip_shapes(self.text);
                for (p, _) in &mut shapes {
                    p.apply_affine(xf);
                }
                let region = unite(&shapes);
                let inner = match (region, clip) {
                    (Some(r), Some(c)) => boolean_step(&r, c, BoolOp::Intersect).ok(),
                    (Some(r), None) => Some(r),
                    (None, _) => None,
                };
                // A clipping path with no outline shows nothing.
                let Some(inner) = inner else { return };
                // As the renderer paints it (`Node::clip_paint`): the clipping path's fills behind
                // the clipped art, inside the clip; its strokes over it, not clipped by it.
                let paint = if first.visible && first.opacity > 0.0 { first.clip_paint() } else { Default::default() };
                if let Some(fill) = &paint.fill {
                    self.once(fill, xf, Some(&inner), depth + 1);
                }
                self.children(rest, xf, Some(&inner), depth);
                if let Some(stroke) = &paint.stroke {
                    self.once(stroke, xf, clip, depth + 1);
                }
            }
            NodeKind::Group { .. } | NodeKind::Layer { .. } => {
                self.group_paint(n);
                self.children(n.editing_children(), xf, clip, depth);
            }
            NodeKind::Text(_) => match self.text.and_then(|f| f(n)) {
                Some(outlined) => self.once(&outlined, xf, clip, depth + 1),
                None => self.warn("type isn't cut; outline it first (Type › Create Outlines)"),
            },
            NodeKind::Image(_) => self.warn("images aren't cut; trace them first (Object › Image Trace)"),
            NodeKind::SymbolInstance { .. } => self.warn("symbol instances aren't cut; break the link first"),
            // Live kinds (placed documents among them) are expanded above; one that `is_live`
            // doesn't own is left out.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::Intertwine { .. }
            | NodeKind::PlacedDocument(_) => {
                self.warn("live art that doesn't expand isn't cut");
            }
        }
    }

    fn children(&mut self, kids: &[std::sync::Arc<Node>], xf: Affine, clip: Option<&BezPath>, depth: usize) {
        for c in kids {
            self.node(c, xf, clip, depth + 1);
        }
    }

    /// A group's or layer's own fills and strokes aren't cut (its members are).
    fn group_paint(&mut self, n: &Node) {
        if n.appearance.items.iter().any(|i| i.visible() && !matches!(i.paint(), Fill::None)) {
            self.warn("a group's own fills and strokes aren't cut; its members are");
        }
    }

    /// The fills and strokes of a path or compound path, `bp` in document space read by `rule`,
    /// in appearance order.
    fn appearance(&mut self, n: &Node, bp: &BezPath, rule: FillRule, closed: bool, xf: Affine, clip: Option<&BezPath>) {
        if bp.elements().is_empty() {
            return;
        }
        for item in &n.appearance.items {
            if self.full {
                return;
            }
            if !item.visible() || item.opacity() <= 0.0 {
                continue;
            }
            let Some((colour, gradient)) = self.colour(item.paint(), item.opacity() * n.opacity) else { continue };
            let area = match item {
                AppearanceItem::Fill(_) => {
                    let mut p = bp.clone();
                    p.apply_affine(xf);
                    normalised(p, rule)
                }
                AppearanceItem::Stroke(s) => {
                    let Some(area) = self.stroke(s, bp, rule, closed, xf) else { continue };
                    area
                }
            };
            let source = Source { node: n.id, name: n.name.clone(), is_stroke: !item.is_fill(), gradient };
            self.push(area, colour, clip, source);
        }
    }

    /// The area stroke `s` paints along `bp` (document space, placed by `xf`), non-zero.
    fn stroke(&mut self, s: &StrokeLayer, bp: &BezPath, rule: FillRule, closed: bool, xf: Affine) -> Option<BezPath> {
        if !(s.width.is_finite() && s.width > 0.0) {
            return None;
        }
        if s.dash.as_ref().is_some_and(|d| d.pattern.iter().any(|v| *v > 0.0)) {
            self.warn("dashed strokes are cut solid");
        }
        if s.profile.is_some() || s.brush.is_some() || s.start_arrow.is_some() || s.end_arrow.is_some() {
            self.warn("width profiles, brushes and arrowheads are cut as plain strokes");
        }
        let mut spine = bp.clone();
        spine.apply_affine(xf);
        // A stroke's weight scales with the copy (as the copy draws it). Capped before and after
        // (a weight near 1e200 from a file would keep the stroker subdividing arcs).
        let scale = xf.determinant().abs().sqrt();
        let width = s.width.min(MAX_STROKE_WIDTH) * if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let width = width.min(MAX_STROKE_WIDTH);
        let align = if closed { s.align } else { StrokeAlign::Center };
        let full = if align == StrokeAlign::Center { width } else { 2.0 * width };
        let cap = match s.cap {
            LineCap::Butt => vectorcraft_pathops::Cap::Butt,
            LineCap::Round => vectorcraft_pathops::Cap::Round,
            LineCap::Square => vectorcraft_pathops::Cap::Square,
        };
        let join = match s.join {
            LineJoin::Miter => vectorcraft_pathops::Join::Miter,
            LineJoin::Round => vectorcraft_pathops::Join::Round,
            LineJoin::Bevel => vectorcraft_pathops::Join::Bevel,
        };
        let miter = if s.miter_limit.is_finite() { s.miter_limit } else { 4.0 };
        let ring = vectorcraft_pathops::outline_stroke(&PathData::from_bezpath(&spine), full, cap, join, miter).to_bezpath();
        if ring.elements().is_empty() {
            return None;
        }
        let op = match align {
            StrokeAlign::Center => return Some(ring),
            StrokeAlign::Inside => BoolOp::Intersect,
            StrokeAlign::Outside => BoolOp::Subtract,
        };
        let fill = normalised(spine, rule);
        match boolean_step(&ring, &fill, op) {
            Ok(p) => Some(p),
            Err(_) => {
                self.warn("a stroke couldn't be aligned; it is cut centred");
                Some(ring)
            }
        }
    }

    /// One painted area in `colour` (document space, non-zero), cut to `clip`.
    fn push(&mut self, mut path: BezPath, colour: [f32; 4], clip: Option<&BezPath>, source: Source) {
        if path.elements().is_empty() || self.full {
            return;
        }
        if self.out.paints.len() > MAX_PAINTS {
            // `compute` refuses past its cap with a warning: one more is enough to say so.
            self.full = true;
            return;
        }
        if let Some(c) = clip {
            match boolean_step(&path, c, BoolOp::Intersect) {
                Ok(p) => path = p,
                Err(_) => self.warn("a shape couldn't be clipped; it is cut whole"),
            }
        }
        if path.elements().is_empty() {
            return;
        }
        let order = u32::try_from(self.out.paints.len()).unwrap_or(u32::MAX);
        self.out.paints.push(Paint { path, colour, order });
        self.out.sources.push(source);
    }

    /// The colour a fill or stroke paints, sRGB 0–1 with `opacity` as alpha, and whether it is a
    /// gradient's mean colour. `None`: it paints nothing the stencil can cut.
    fn colour(&mut self, paint: &Fill, opacity: f32) -> Option<([f32; 4], bool)> {
        match paint {
            Fill::None => None,
            Fill::Solid { color, .. } => {
                let [r, g, b] = rgb(color);
                Some(([r, g, b, opacity], false))
            }
            Fill::Gradient(g) => {
                self.warn("gradients are cut as their mean colour");
                let stops = &g.gradient.stops;
                let n = stops.len().max(1) as f32;
                let sum = stops.iter().fold([0.0f32; 4], |a, s| {
                    let [r, g, b] = rgb(&s.color);
                    [a[0] + r, a[1] + g, a[2] + b, a[3] + s.opacity]
                });
                if stops.is_empty() {
                    return None;
                }
                Some(([sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n * opacity], true))
            }
            Fill::Pattern { .. } => {
                self.warn("patterns aren't cut; they're left out");
                None
            }
        }
    }
}

/// `color` as display sRGB 0–1 (finite, clamped).
fn rgb(color: &Color) -> [f32; 3] {
    color.to_rgb().map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })
}

/// `p` as plain non-zero contours (even-odd paths simplified by their rule).
fn normalised(p: BezPath, rule: FillRule) -> BezPath {
    if rule == FillRule::EvenOdd { simplify_with_rule(&p, true).unwrap_or(p) } else { p }
}

/// The union of `shapes` (each read by its own rule) as plain contours; `None` when there are
/// none or the union fails.
fn unite(shapes: &[(BezPath, FillRule)]) -> Option<BezPath> {
    let mut all = BezPath::new();
    for (p, rule) in shapes {
        let plain = simplify_with_rule(p, *rule == FillRule::EvenOdd).ok()?;
        all.extend(plain.elements().iter().copied());
    }
    if all.elements().is_empty() {
        return None;
    }
    crate::ops::simplify(&all).ok()
}
