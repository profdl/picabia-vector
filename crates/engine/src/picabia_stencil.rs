//! PICABIA: stencil objects, engine side: computing their sheets and keeping them fresh.
//!
//! An object whose modifier stack has an enabled `stencil` row draws as a laser-cut katagami
//! sheet instead of its design. The geometry is `vectorcraft_stencil`'s (gather, compute); the
//! results live in [`vectorcraft_stencil::store`] (same layer as the renderer, which reads them
//! there). Here:
//! - [`compute_now`]: gather, compute and keep one object's sheet (or re-key a kept one whose
//!   design is unchanged: [`vectorcraft_stencil::store::rekey`]).
//! - [`poll`] (the app, every frame) / [`refresh`]: bring every stale stencil of a document up to
//!   date without blocking: natively each on a worker thread (at most [`MAX_JOBS`] at once), on
//!   wasm (no threads) on the calling thread once the object has stopped changing for
//!   [`SETTLE_SECS`]. Meanwhile the latest result keeps drawing. [`busy`] says one is coming.
//! - [`ensure`] / [`ensure_one`]: compute now, on the calling thread (exports, commands, headless).
//! - [`stencil_sheet`], [`generation`]: what the renderer reads.
//! - [`info_json`]: the `modifier.stencilInfo` shape.

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_geom::kurbo::{BezPath, PathEl, Point, Shape as _};
use vectorcraft_stencil::store::{self, Inputs};
pub use vectorcraft_stencil::store::{Drawn, Fill, Frame, Info, Sheet, drawn, fills, frame, generation, is_stencil};
use vectorcraft_stencil::{Island, Problem, ProblemKind, Stats};

/// Background computes running at once (native).
pub const MAX_JOBS: usize = 2;
/// wasm computes on the UI thread: only once an object has stopped changing for this long.
pub const SETTLE_SECS: f64 = 0.3;
/// The most stencil objects of one document kept up to date in the background.
const MAX_STENCILS: usize = 256;

/// Object `n`'s stencil sheet in `doc` (fresh or the latest, being recomputed); `None` when it
/// isn't a stencil or nothing was computed for it yet (draw its design). What the renderer draws
/// in place of the design ([`fills`] paints it as the preview params say); [`generation`] moves
/// whenever a result lands.
pub fn stencil_sheet(doc: &Document, n: &Node) -> Option<Arc<Sheet>> {
    if !is_stencil(n) || frame(doc, n).is_none() {
        return None;
    }
    store::lookup(doc, n).map(|(s, _)| s)
}

/// Object `n`'s latest stencil and whether it was computed from what `doc` holds now.
pub fn lookup(doc: &Document, n: &Node) -> Option<(Arc<Sheet>, bool)> {
    store::lookup(doc, n)
}

/// The stencil objects of `doc` (at most [`MAX_STENCILS`]).
pub fn stencil_objects(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    for l in &doc.layers {
        l.walk(&mut |n| {
            if out.len() < MAX_STENCILS && is_stencil(n) {
                out.push(n.id);
            }
        });
    }
    out
}

/// The key of a result that says the object has no size to cut.
const NO_SIZE_KEY: u64 = 0x6e6f_7369_7a65;
/// The key of a result whose compute failed (a bug in the geometry).
const FAILED_KEY: u64 = 0x6661_696c_6564;

/// Test hook: the compute of an object with this name panics.
#[cfg(test)]
pub(crate) const PANIC_NAME: &str = "__picabia_stencil_panic_test__";

/// Compute object `id`'s stencil now (or re-key a kept result for the same design) and keep it.
/// `None`: not a stencil object, or no size to cut. Whatever happens a result keyed by what the
/// document holds now is kept (an object with no size, or whose compute fails, keeps an empty
/// one), so the object counts as fresh and the background refresh rests until it changes.
pub fn compute_now(doc: &Document, id: NodeId) -> Option<Arc<Sheet>> {
    let node = doc.node_arc(id)?;
    if !is_stencil(node) {
        return None;
    }
    let inputs = Inputs::take(doc, node);
    let Some(frame) = frame(doc, node) else {
        let info = Info { warnings: vec!["the object has no size to cut".into()], ..Info::default() };
        store::store(id, inputs, NO_SIZE_KEY, Sheet { info, ..Sheet::default() });
        return None;
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute_keyed(doc, node, &frame, inputs.clone())));
    match run {
        Ok(sheet) => Some(sheet),
        Err(_) => {
            let info = Info {
                modifier: frame.modifier.clone(),
                warnings: vec!["the stencil couldn't be computed (an internal error); change the design or its settings to try again".into()],
                ..Info::default()
            };
            let sheet = Sheet { cut: BezPath::new(), paper: frame.sheet.to_path(0.1), sheet: frame.sheet, info };
            Some(store::store(id, inputs, FAILED_KEY, sheet))
        }
    }
}

/// Gather, then re-key a kept result with the same geometry, or compute and keep a new one.
fn compute_keyed(doc: &Document, node: &Node, frame: &Frame, inputs: Inputs) -> Arc<Sheet> {
    #[cfg(test)]
    if node.name.as_deref() == Some(PANIC_NAME) {
        #[allow(clippy::panic)]
        {
            panic!("injected stencil compute panic");
        }
    }
    let text = vectorcraft_render::effects::text_outliner();
    // Geometry rows above the stencil reshape what it cuts: the copies as drawn, taken from the
    // renderer's kept art (reshaped once, shared with the canvas; the key below is computed from
    // what is gathered, so they can't be skipped before it).
    let shaped = vectorcraft_modifiers::node_instances_in_doc(doc, node)
        .filter(|o| o.has_geometry())
        .and_then(|_| vectorcraft_render::picabia::art(doc, node));
    let g = match shaped {
        Some(art) => {
            let copies: Vec<Node> = art.children().into_iter().flatten().map(|c| Node::clone(c)).collect();
            let mut g = vectorcraft_stencil::gather_shaped(doc, &copies, text);
            g.warnings.extend(vectorcraft_render::picabia::notes(doc, node));
            g
        }
        None => vectorcraft_stencil::gather(doc, node, &frame.copies, text),
    };
    let key = store::geometry_key(frame, &g);
    if let Some(kept) = store::rekey(node.id, key, inputs.clone()) {
        return kept;
    }
    let sheet = compute_sheet(frame, g);
    store::store(node.id, inputs, key, sheet)
}

/// The sheet of `frame` for the gathered design `g`; a failed compute is an uncut sheet with the
/// reason as a warning.
fn compute_sheet(frame: &Frame, g: vectorcraft_stencil::Gathered) -> Sheet {
    let mut warnings = g.warnings;
    let design_sheet = |mpu: f64| {
        let m = frame.params.margin_mm / mpu;
        let s = frame.layer_bounds.abs().inflate(m, m);
        (s.width() * mpu, s.height() * mpu)
    };
    let paints = g.paints.len();
    match vectorcraft_stencil::compute_repeat(&g.paints, &frame.params, frame.layer_bounds, frame.repeat) {
        Ok(out) => {
            warnings.extend(out.warnings);
            let mpu = out.mm_per_unit;
            let cell = out.cell.as_ref().and_then(corners);
            let info = Info {
                modifier: frame.modifier.clone(),
                colours: out.colours,
                warnings,
                stats: out.stats,
                sheet_mm: design_sheet(mpu),
                tile_mm: out.tile.as_ref().map(|t| (t.sheet.width() * mpu, t.sheet.height() * mpu)),
                paints,
                islands: out.islands,
                ties: out.ties,
                problems: out.problems,
                hoshi: out.hoshi,
                repeat: if cell.is_some() { frame.repeat } else { None },
                cell,
            };
            Sheet { cut: out.cut, paper: out.paper, sheet: out.sheet, info }
        }
        Err(e) => {
            warnings.push(e.to_string());
            let info = Info { modifier: frame.modifier.clone(), warnings, sheet_mm: design_sheet(frame.mm_per_unit), paints, ..Info::default() };
            Sheet { cut: BezPath::new(), paper: frame.sheet.to_path(0.1), sheet: frame.sheet, info }
        }
    }
}

/// The four corners of a repeat cell's outline (a closed quadrilateral).
fn corners(p: &BezPath) -> Option<[Point; 4]> {
    let pts: Vec<Point> = p
        .elements()
        .iter()
        .filter_map(|e| match e {
            PathEl::MoveTo(q) | PathEl::LineTo(q) => Some(*q),
            _ => None,
        })
        .collect();
    let pts = match pts.as_slice() {
        [a, .., z] if pts.len() == 5 && a == z => pts.get(..4)?,
        _ => pts.as_slice(),
    };
    match pts {
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

/// Object `id`'s stencil, fresh: the kept one when it matches `doc`, else computed now. `None`:
/// not a stencil, or no size to cut (a "no size" result is still kept, so it counts as fresh).
pub fn ensure_one(doc: &Document, id: NodeId) -> Option<Arc<Sheet>> {
    let n = doc.node(id)?;
    if !is_stencil(n) {
        return None;
    }
    if frame(doc, n).is_none() {
        if !store::is_fresh(doc, n) {
            compute_now(doc, id);
        }
        return None;
    }
    match store::lookup(doc, n) {
        Some((s, true)) => Some(s),
        _ => compute_now(doc, id),
    }
}

/// Compute every stale stencil of `doc` now (exports and headless renders draw fresh sheets).
pub fn ensure(doc: &Document) {
    for id in stencil_objects(doc) {
        ensure_one(doc, id);
    }
}

fn is_stale(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| !store::is_fresh(doc, n))
}

#[derive(Default)]
struct Sched {
    /// Objects being computed in the background.
    running: Vec<NodeId>,
    /// The document last refreshed and its stencil objects (found again only when it changes).
    seen: Option<(Weak<Document>, Vec<NodeId>)>,
    /// wasm: stale objects waiting to settle (the document they were seen in, since when).
    #[cfg(target_arch = "wasm32")]
    waiting: Vec<(NodeId, Weak<Document>, f64)>,
}

static SCHED: Mutex<Option<Sched>> = Mutex::new(None);

fn sched() -> MutexGuard<'static, Option<Sched>> {
    // A panic elsewhere while holding the lock leaves plain data: keep using it.
    SCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether a stencil result is still coming (keep repainting until it lands).
pub fn busy() -> bool {
    let s = sched();
    let Some(s) = s.as_ref() else { return false };
    #[cfg(target_arch = "wasm32")]
    if !s.waiting.is_empty() {
        return true;
    }
    !s.running.is_empty()
}

/// The stencil objects of `doc`, found again only when the document changed since the last call.
fn objects_cached(doc: &Arc<Document>) -> Vec<NodeId> {
    if let Some(s) = sched().as_ref()
        && let Some((w, ids)) = &s.seen
        && std::ptr::eq(w.as_ptr(), Arc::as_ptr(doc))
    {
        return ids.clone();
    }
    let ids = stencil_objects(doc);
    sched().get_or_insert_with(Sched::default).seen = Some((Arc::downgrade(doc), ids.clone()));
    ids
}

/// The app's per-frame hook: [`refresh`] the active document of `s`; `now` is the app's clock in
/// seconds (wasm's settle reads it). Returns [`busy`].
pub fn poll(s: &crate::Session, now: f64) -> bool {
    if let Ok(st) = s.doc() {
        refresh(&st.doc, now);
    }
    busy()
}

/// Bring `doc`'s stencils up to date without blocking the caller long: stale ones are recomputed
/// in the background (native) or once they've stopped changing for [`SETTLE_SECS`] of `now`
/// (wasm). Cheap when nothing is stale: pointer comparisons.
pub fn refresh(doc: &Arc<Document>, now: f64) {
    let stale: Vec<NodeId> = objects_cached(doc).into_iter().filter(|id| is_stale(doc, *id)).collect();
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = sched().as_mut() {
        s.waiting.retain(|w| stale.contains(&w.0));
    }
    for id in stale {
        schedule(doc, id, now);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn schedule(doc: &Arc<Document>, id: NodeId, _now: f64) {
    {
        let mut g = sched();
        let s = g.get_or_insert_with(Sched::default);
        if s.running.contains(&id) || s.running.len() >= MAX_JOBS {
            return;
        }
        s.running.push(id);
    }
    /// Takes the object off the running list however the job ends (a panic too).
    struct Done(NodeId);
    impl Drop for Done {
        fn drop(&mut self) {
            if let Some(s) = sched().as_mut() {
                s.running.retain(|r| *r != self.0);
            }
        }
    }
    let doc2 = doc.clone();
    let spawned = std::thread::Builder::new().name("picabia-stencil".into()).spawn(move || {
        let _done = Done(id);
        // A panic in the geometry ends this job only; the canvas keeps the latest result.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute_now(&doc2, id)));
    });
    if spawned.is_err() {
        // No thread to be had: compute here (the caller waits once).
        let _done = Done(id);
        compute_now(doc, id);
    }
}

#[cfg(target_arch = "wasm32")]
fn schedule(doc: &Arc<Document>, id: NodeId, now: f64) {
    // An object with nothing to draw yet computes at once.
    let first = doc.node(id).is_some_and(|n| store::lookup(doc, n).is_none());
    let ready = {
        let mut g = sched();
        let s = g.get_or_insert_with(Sched::default);
        match s.waiting.iter_mut().find(|w| w.0 == id) {
            Some(w) if std::ptr::eq(w.1.as_ptr(), Arc::as_ptr(doc)) => first || !(now - w.2 < SETTLE_SECS),
            Some(w) => {
                *w = (id, Arc::downgrade(doc), now);
                first
            }
            None => {
                s.waiting.push((id, Arc::downgrade(doc), now));
                first
            }
        }
    };
    if ready {
        compute_now(doc, id);
        if let Some(s) = sched().as_mut() {
            s.waiting.retain(|w| w.0 != id);
        }
    }
}

/// Wait (at most `secs`) for the background computes to finish (tests).
#[cfg(not(target_arch = "wasm32"))]
pub fn wait_idle(secs: f64) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs.clamp(0.0, 600.0));
    while busy() {
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    true
}

/// Every stencil material preset (`custom` has none) as `modifier.stencilInfo` lists them.
pub fn materials_json() -> Value {
    let rows: Vec<Value> = vectorcraft_modifiers::StencilMaterial::ALL
        .iter()
        .filter_map(|m| {
            let l = m.limits()?;
            Some(json!({
                "id": m.id(),
                "minCutWidthMm": l.min_cut_width_mm,
                "minBridgeMm": l.min_bridge_mm,
                "tieWidthMm": l.tie_width_mm,
                "minCornerAngle": l.min_corner_angle,
                "kerfMm": l.kerf_mm,
            }))
        })
        .collect();
    Value::Array(rows)
}

/// A number for JSON (what isn't finite reads 0).
fn num(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { json!(0.0) }
}

fn stats_json(s: &Stats) -> Value {
    json!({
        "cutPieces": s.cut_pieces,
        "droppedSpecks": s.dropped_specks,
        "filledHoles": s.filled_holes,
        "cutLengthMm": num(s.cut_length_mm),
        "areaCutMm2": num(s.area_cut_mm2),
        "islands": s.islands,
        "ties": s.ties,
        "untied": s.untied,
        "patternHeld": s.pattern_held,
        "repeatIslands": s.repeat_islands,
        "problems": ProblemKind::ALL.iter().map(|k| (k.id().to_string(), json!(s.problems_by_kind.get(*k)))).collect::<Map<_, _>>(),
    })
}

/// One problem as `modifier.stencilInfo` lists it: its kind and bounds (document space).
fn problem_json(p: &Problem) -> Value {
    let b = p.bounds;
    json!({"kind": p.kind.id(), "x": num(b.x0), "y": num(b.y0), "w": num(b.width()), "h": num(b.height())})
}

/// One island as `modifier.stencilInfo` lists it: its bounds (document space), the ties holding
/// it, whether a tie pattern holds it and how deep it sits (0: in a cut-out of the sheet).
fn island_json(i: &Island) -> Value {
    let b = i.bounds;
    json!({"x": num(b.x0), "y": num(b.y0), "w": num(b.width()), "h": num(b.height()), "tied": i.tied, "patternHeld": i.pattern_held, "depth": i.nested_depth})
}

/// `info` as `modifier.stencilInfo` returns it.
pub fn info_json(info: &Info) -> Value {
    json!({
        "stencil": info.modifier,
        "colours": info.colours.iter().map(|c| json!({"hex": c.hex, "cut": c.cut, "area": num(c.area)})).collect::<Vec<_>>(),
        "warnings": info.warnings,
        "stats": stats_json(&info.stats),
        "islands": info.islands.iter().map(island_json).collect::<Vec<_>>(),
        "problems": info.problems.iter().map(problem_json).collect::<Vec<_>>(),
        "hoshi": info.hoshi.iter().map(|p| json!({"x": num(p.x), "y": num(p.y)})).collect::<Vec<_>>(),
        "repeat": info.repeat.map_or(Value::Null, |r| json!({"ax": num(r.a.x), "ay": num(r.a.y), "bx": num(r.b.x), "by": num(r.b.y)})),
        "cell": info.cell.map_or(Value::Null, |c| Value::Array(c.iter().map(|p| json!({"x": num(p.x), "y": num(p.y)})).collect())),
        "tile": info.tile_mm.map_or(Value::Null, |(w, h)| json!({"w": num(w), "h": num(h)})),
        "sheetMm": {"w": num(info.sheet_mm.0), "h": num(info.sheet_mm.1)},
        "materials": materials_json(),
        "shapes": info.paints,
    })
}
