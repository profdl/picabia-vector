//! PICABIA: laser-cut any design, not just stencils: `laser.gcode`. The objects' visible
//! outlines are gathered in document space (paths, compound paths, type as glyph outlines, live
//! art expanded, modifier copies; groups and layers by their contents), converted to mm and made
//! one [`LaserSheet`]: the page is the objects' bounds, origin top-left. Closed outlines are cut
//! (cleaned, inner contours first, nearest neighbour, the largest outermost last:
//! [`vectorcraft_stencil::order_contours`]); open paths are cut as open polylines with the score
//! settings. No kerf compensation and no sheet outline. A single stencil object keeps the
//! stencil path (`modifier.stencilGcode`).
//!
//! Text only: the G-code is written to a file or returned, never sent to a machine.

use serde_json::{Value, json};
use vectorcraft_doc::live::{Outliner, expand_deep, is_live};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_modifiers::StencilMaterial;
use vectorcraft_stencil::{Contour, LaserSheet, Polyline};

use super::laser::{Out, bad, file_safe, get, out_of};
use super::laser_gcode::{MachineProfile, bed_mm, emit_one, job_params, material_param, remember_ops};
use crate::{EngineError, Result, Session};

pub(super) const LASER_GCODE: &str = "laser.gcode";

/// Millimetres per document unit by default: a point (1/72 in), VectorCraft's unit.
pub const DEFAULT_MM_PER_UNIT: f64 = 25.4 / 72.0;
/// The biggest design `laser.gcode` cuts, mm a side.
pub const MAX_DESIGN_MM: f64 = 20_000.0;
/// Curves are flattened to within this (mm).
const FLATTEN_MM: f64 = 0.01;
/// Objects visited at most.
const MAX_VISITS: usize = 100_000;
/// Nesting followed at most.
const MAX_DEPTH: usize = 64;
/// Points kept at most (the G-code writer has its own line cap).
const MAX_POINTS: usize = 2_000_000;

/// A design ready to cut.
struct DesignSheet {
    sheet: LaserSheet,
    warnings: Vec<String>,
    mm_per_unit: f64,
    /// A name for files and jobs.
    base: String,
}

/// The single stencil object `p` names, if that is what it names.
fn single_stencil(s: &Session, p: &Value) -> Option<NodeId> {
    let ids = super::super::targets(s, p).ok()?;
    let [id] = ids.as_slice() else { return None };
    let n = s.doc().ok()?.doc.node(*id)?;
    crate::picabia_stencil::is_stencil(n).then_some(*id)
}

/// Millimetres per unit: `widthMm` (the objects' bounds scaled to that width) wins over
/// `mmPerUnit`; default [`DEFAULT_MM_PER_UNIT`].
fn scale(cmd: &str, p: &Value, bounds: Rect) -> Result<f64> {
    if let Some(v) = get(p, "widthMm") {
        let w =
            v.as_f64().filter(|w| w.is_finite() && *w > 0.0 && *w <= 5000.0).ok_or_else(|| bad(cmd, "`widthMm` must be a width in mm (0-5000)"))?;
        return vectorcraft_stencil::mm_per_unit(w, bounds)
            .ok_or_else(|| EngineError::Disabled(cmd.into(), "the objects have no width to scale".into()));
    }
    match get(p, "mmPerUnit") {
        None => Ok(DEFAULT_MM_PER_UNIT),
        Some(v) => v.as_f64().filter(|m| m.is_finite() && *m >= 0.001 && *m <= 100.0).ok_or_else(|| bad(cmd, "`mmPerUnit` must be 0.001-100")),
    }
}

/// The objects `p` names (`ids`, default the selection) as one sheet.
fn design_sheet(cmd: &str, s: &Session, p: &Value) -> Result<DesignSheet> {
    let doc = &s.doc()?.doc;
    let ids = super::super::targets(s, p).map_err(|e| bad(cmd, e.to_string()))?;
    if ids.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "select the objects to cut".into()));
    }
    let mut roots = Vec::new();
    for id in &ids {
        let n = doc.node(*id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
        roots.push(n);
    }
    let text = vectorcraft_render::effects::text_outliner();
    let mut g = Gather { doc, text, closed: Vec::new(), open: Vec::new(), warnings: Vec::new(), visits: 0, points: 0, tol: 0.1 };
    // Gathered once at a fine tolerance to find the bounds, then flattened to the mm tolerance.
    let mut paths: Vec<BezPath> = Vec::new();
    for n in &roots {
        g.node(n, Affine::IDENTITY, 0, &mut paths);
    }
    // Outlines that aren't finite everywhere are left out.
    paths.retain(|p| {
        let b = vectorcraft_geom::kurbo::Shape::bounding_box(p);
        [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())
    });
    let bounds = paths.iter().map(vectorcraft_geom::kurbo::Shape::bounding_box).reduce(|a, b| a.union(b));
    let Some(bounds) = bounds else {
        let why = g.warnings.first().map(|w| format!(" ({w})")).unwrap_or_default();
        return Err(EngineError::Disabled(cmd.into(), format!("the objects have no outlines to cut{why}")));
    };
    let mpu = scale(cmd, p, bounds)?;
    // Refused before flattening: a huge design would flatten into an enormous number of points.
    let (w_mm, h_mm) = (bounds.width() * mpu, bounds.height() * mpu);
    if !(w_mm.is_finite() && h_mm.is_finite() && w_mm <= MAX_DESIGN_MM && h_mm <= MAX_DESIGN_MM) {
        return Err(EngineError::Disabled(
            cmd.into(),
            format!("the design would be {w_mm:.0} × {h_mm:.0} mm; at most {MAX_DESIGN_MM:.0} mm a side is cut (scale it with widthMm or mmPerUnit)"),
        ));
    }
    g.tol = (FLATTEN_MM / mpu).max(1e-6);
    for path in &paths {
        g.flatten(path);
    }
    // Document space → mm, origin at the bounds' top-left.
    let to_mm = |pts: Vec<Point>| -> Vec<Point> { pts.into_iter().map(|q| Point::new((q.x - bounds.x0) * mpu, (q.y - bounds.y0) * mpu)).collect() };
    let cut: Vec<Contour> = vectorcraft_stencil::order_contours(g.closed.drain(..).map(to_mm).collect());
    let score: Vec<Polyline> = order_open(g.open.drain(..).map(to_mm).filter(|l| l.len() >= 2).collect());
    if cut.is_empty() && score.is_empty() {
        let why = g.warnings.first().map(|w| format!(" ({w})")).unwrap_or_default();
        return Err(EngineError::Disabled(cmd.into(), format!("the objects have no outlines to cut{why}")));
    }
    let first = roots.first().and_then(|n| n.name.clone()).unwrap_or_default();
    let base = file_safe(if ids.len() == 1 { &first } else { "Design" });
    let sheet = LaserSheet {
        w_mm: bounds.width() * mpu,
        h_mm: bounds.height() * mpu,
        cut,
        score,
        engrave: Vec::new(),
        name: "design".into(),
        loose_pieces: 0,
    };
    Ok(DesignSheet { sheet, warnings: g.warnings, mm_per_unit: mpu, base })
}

/// Open polylines in a short-travel order (nearest end first; a line may run backwards).
fn order_open(mut lines: Vec<Polyline>) -> Vec<Polyline> {
    let mut out = Vec::with_capacity(lines.len());
    let mut head = Point::ZERO;
    while !lines.is_empty() {
        let mut best = (0, false, f64::INFINITY);
        // Past a few thousand lines, take them as they come.
        if out.len() <= 5000 {
            for (i, l) in lines.iter().enumerate() {
                if let (Some(&a), Some(&b)) = (l.first(), l.last()) {
                    let (da, db) = ((a - head).hypot2(), (b - head).hypot2());
                    if da < best.2 {
                        best = (i, false, da);
                    }
                    if db < best.2 {
                        best = (i, true, db);
                    }
                }
            }
        }
        let mut l = lines.swap_remove(best.0.min(lines.len() - 1));
        if best.1 {
            l.reverse();
        }
        if let Some(&e) = l.last() {
            head = e;
        }
        out.push(l);
    }
    out
}

struct Gather<'a> {
    doc: &'a Document,
    text: Outliner<'a>,
    closed: Vec<Vec<Point>>,
    open: Vec<Vec<Point>>,
    warnings: Vec<String>,
    visits: usize,
    points: usize,
    /// Flattening tolerance, document units.
    tol: f64,
}

impl Gather<'_> {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// `n` placed by `xf` (document space to document space), each modifier copy.
    fn node(&mut self, n: &Node, xf: Affine, depth: usize, out: &mut Vec<BezPath>) {
        if !n.visible || n.is_template() {
            return;
        }
        match vectorcraft_modifiers::node_instances_in_doc(self.doc, n) {
            Some(o) if !o.instances.is_empty() => {
                for t in o.transforms() {
                    self.once(n, xf * t, depth, out);
                }
            }
            _ => self.once(n, xf, depth, out),
        }
    }

    fn once(&mut self, n: &Node, xf: Affine, depth: usize, out: &mut Vec<BezPath>) {
        self.visits += 1;
        if depth > MAX_DEPTH || self.visits > MAX_VISITS {
            self.warn("the design is too large or nested too deep; part of it is left out");
            return;
        }
        if !xf.is_finite() {
            return;
        }
        if is_live(n) {
            let expanded = expand_deep(n, self.text);
            if is_live(&expanded) {
                self.warn("live art that doesn't expand isn't cut");
            } else {
                self.once(&expanded, xf, depth + 1, out);
            }
            return;
        }
        match &n.kind {
            NodeKind::Path { guide: true, .. } => {}
            NodeKind::Path { path, .. } => self.push(path.to_bezpath(), xf, out),
            NodeKind::Compound { children, .. } => {
                for p in children.iter().filter_map(|c| c.path_data()) {
                    self.push(p.to_bezpath(), xf, out);
                }
            }
            NodeKind::Group { clip, .. } | NodeKind::Layer { clip, .. } => {
                if *clip {
                    self.warn("clipping isn't applied: clipped objects are cut whole");
                }
                for c in n.editing_children() {
                    self.node(c, xf, depth + 1, out);
                }
            }
            NodeKind::Text(_) => match self.text.and_then(|f| f(n)) {
                Some(outlined) => self.once(&outlined, xf, depth + 1, out),
                None => self.warn("a type object couldn't be outlined; it is left out"),
            },
            NodeKind::Image(_) => self.warn("images aren't cut; trace them first (Object › Image Trace)"),
            NodeKind::SymbolInstance { .. } => self.warn("symbol instances aren't cut; break the link first"),
            _ => self.warn("live art that doesn't expand isn't cut"),
        }
    }

    fn push(&mut self, mut p: BezPath, xf: Affine, out: &mut Vec<BezPath>) {
        if self.points > MAX_POINTS {
            self.warn("the design is too large or nested too deep; part of it is left out");
            return;
        }
        self.points += p.elements().len();
        p.apply_affine(xf);
        out.push(p);
    }

    /// `p` (document space) flattened: closed subpaths and open ones.
    fn flatten(&mut self, p: &BezPath) {
        let mut cur: Vec<Point> = Vec::new();
        let done = |cur: &mut Vec<Point>, closed: bool, g: &mut Self| {
            let pts = std::mem::take(cur);
            let closes = closed || (pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| (*a - *b).hypot() < 1e-9));
            g.points += pts.len();
            if g.points > MAX_POINTS * 2 {
                g.warn("the design is too large or nested too deep; part of it is left out");
                return;
            }
            if pts.len() >= 2 && pts.iter().all(|q| q.x.is_finite() && q.y.is_finite()) {
                if closes { g.closed.push(pts) } else { g.open.push(pts) }
            }
        };
        let mut els: Vec<PathEl> = Vec::new();
        vectorcraft_geom::kurbo::flatten(p, self.tol, |el| els.push(el));
        for el in els {
            match el {
                PathEl::MoveTo(q) => {
                    done(&mut cur, false, self);
                    cur.push(q);
                }
                PathEl::LineTo(q) => cur.push(q),
                PathEl::ClosePath => done(&mut cur, true, self),
                _ => {}
            }
        }
        done(&mut cur, false, self);
    }
}

/// `laser.gcode`: any objects as one G-code file (a single stencil object: as
/// `modifier.stencilGcode`).
pub(super) fn gcode(s: &mut Session, p: &Value) -> Result<Value> {
    if single_stencil(s, p).is_some() {
        return super::laser_gcode::stencil_gcode(s, p);
    }
    for k in ["kerfMm", "overlapMm", "label", "bedWidthMm", "bedHeightMm"] {
        if get(p, k).is_some() {
            return Err(bad(LASER_GCODE, format!("`{k}` is for stencil objects only")));
        }
    }
    let out = out_of(LASER_GCODE, p)?;
    let material = material_param(LASER_GCODE, p)?.unwrap_or(StencilMaterial::Paper);
    let gp = job_params(s, material, p);
    if out == Out::Dialog {
        let ids: Vec<Value> = super::super::targets(s, p).map_err(|e| bad(LASER_GCODE, e.to_string()))?.into_iter().map(|id| json!(id.0)).collect();
        return Ok(json!({"dialog": "laserGcode", "ids": ids, "material": material.id(), "gcode": gp.to_json()}));
    }
    let d = design_sheet(LASER_GCODE, s, p)?;
    let (w, h) = bed_mm(s);
    let mut warnings = d.warnings.clone();
    if d.sheet.w_mm > w + 0.01 || d.sheet.h_mm > h + 0.01 {
        warnings.push(format!("the design is {:.1} × {:.1} mm, bigger than the bed ({w} × {h} mm)", d.sheet.w_mm, d.sheet.h_mm));
    }
    let rapid = MachineProfile::of(s).rapid_mm_min;
    let files = emit_one(LASER_GCODE, &d.sheet, &gp, &out, &format!("{} laser", d.base), rapid)?;
    if out != Out::Summary {
        remember_ops(s, material, &gp);
    }
    Ok(json!({
        "files": files,
        "sheets": 1,
        "warnings": warnings,
        "material": material.id(),
        "mmPerUnit": d.mm_per_unit,
        "gcode": gp.to_json(),
    }))
}
