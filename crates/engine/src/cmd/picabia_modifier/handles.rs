//! PICABIA: on-canvas handles of the selected object's modifier rows, with the Selection tool.
//!
//! Which rows: with one editable object selected whose stack has enabled rows with handles
//! ([`vectorcraft_modifiers::handles`]), the rows open (or picked) in the Appearance panel show
//! theirs; with none open there, the last enabled row with handles does (picabia-craft's rule).
//! The panel reports its open rows each frame ([`Session::picabia_set_handle_rows`]), for the
//! active document (they're kept by its uid: object ids repeat across documents); the engine
//! can't see egui's memory.
//!
//! A press on a handle (within [`HIT_PX`] screen pixels; the nearest wins, ties going to the later
//! row and, within a row, to the knob listed first; the bounding box's own handles win where they
//! overlap) takes the gesture before the Selection tool sees it, as the Plane Switching Widget
//! does: Alt, Cmd and Ctrl presses stay the tool's (a Shift-press on a knob is the knob's, for its
//! Shift constraint, so Shift-click deselecting doesn't happen on a knob). Dragging previews
//! `modifier.set` inside one interaction, so the drag is one undo step ("Set Modifier"); a drag
//! that ends where it began leaves none; Escape puts the params back. A drag whose document was
//! switched, whose object went, or whose interaction something else ended (an agent's command
//! mid-drag) is dropped: its own preview, if still open in its document, is rolled back. A handle
//! whose field is bound to a parameter or an expression is drawn greyed and doesn't drag: a press
//! on it says why in the status bar. Positions are in document space, so zoom and Rotate View need
//! nothing extra; knobs placed off the box keep
//! [`vectorcraft_modifiers::handles::HANDLE_GAP_PX`] on screen.
//!
//! The handles of a document version, selection, zoom and open rows are built once and kept
//! ([`HandleState`]'s cache): the cursor, hover and drawing of a frame share them.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Point, Rect, Vec2};
use vectorcraft_modifiers::handles::{Frame, HandleKind, Motion, Spot};
use vectorcraft_modifiers::{Guide, Modifier, ModifierKind};
use vectorcraft_tools::{Action, Cursor, Mods, PointerEvent, PointerKind, ToolKey};

use crate::{Result, Session, UiRequest, ViewInfo};

/// How near (screen pixels) a press must come to a handle to take it.
pub const HIT_PX: f64 = 7.0;
/// How near (screen pixels) a bounding-box handle is for it to win over a modifier handle (the
/// Selection tool's own reach for them).
const BOX_HANDLE_PX: f64 = 5.0;

/// What the Appearance panel has open, the hovered handle, the gesture in progress and the kept
/// handles ([`super::PicabiaState`]).
#[derive(Debug, Default)]
pub(crate) struct HandleState {
    /// The document (uid) and object whose rows are open in the panel, and those rows' ids.
    rows: Option<(u64, NodeId, Vec<String>)>,
    /// The handle under the pointer: document uid, object, row id and handle.
    hover: Option<(u64, NodeId, String, HandleKind)>,
    drag: Option<HandleDrag>,
    /// The handles last built and what for (`None`: the object has none to show).
    cache: Mutex<Option<(BaseKey, Option<Arc<Base>>)>>,
    #[cfg(test)]
    builds: std::sync::atomic::AtomicUsize,
}

#[derive(Debug)]
struct HandleDrag {
    /// The document it edits.
    uid: u64,
    id: NodeId,
    modifier: String,
    handle: HandleKind,
    /// The row's params when the drag began (bound fields resolved): the drag measures from them.
    kind: ModifierKind,
    bounds: Rect,
    zoom: f64,
    /// The handle minus the press point: the handle stays where it was grabbed.
    grab: Vec2,
    /// Where the handle was when the drag began.
    start: Point,
    /// Fields the drag mustn't write (bound).
    bound: Vec<String>,
    /// The params the press found, and the last ones previewed.
    from: Map<String, Value>,
    last: Option<Map<String, Value>>,
}

/// One handle as the canvas draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModHandleView {
    /// The row's id and its index in the stack.
    pub modifier: String,
    pub index: usize,
    pub kind: HandleKind,
    pub at: Point,
    /// A field it sets is bound to a parameter or an expression: drawn greyed, not draggable.
    pub bound: bool,
    /// Under the pointer or being dragged.
    pub hot: bool,
}

/// The selected object's modifier handles, their guides and the readout of the one being dragged
/// or hovered (document space).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModHandlesView {
    pub handles: Vec<ModHandleView>,
    pub guides: Vec<Guide>,
    /// The readout and where its handle is; for a bound handle, why it doesn't drag.
    pub readout: Option<(Point, String)>,
}

/// One row whose handles show, with what they need.
#[derive(Debug)]
struct Row {
    index: usize,
    modifier: Modifier,
    /// Its params with bound fields resolved (where the handles sit).
    kind: ModifierKind,
}

/// What the kept handles were built for.
#[derive(Clone, Debug, PartialEq)]
struct BaseKey {
    uid: u64,
    /// Moves with every change of the document or its selection.
    revision: u64,
    zoom: u64,
    id: NodeId,
    open: Option<Vec<String>>,
}

/// The handles of one object, built once per [`BaseKey`].
#[derive(Debug)]
struct Base {
    id: NodeId,
    bounds: Rect,
    rows: Vec<Row>,
    /// Each handle: its row (a position in `rows`), where it is, whether it is bound. In row
    /// order, each row's in [`vectorcraft_modifiers::handles::spots`]' order.
    handles: Vec<(usize, Spot, bool)>,
    guides: Vec<Guide>,
}

fn sane_zoom(z: f64) -> f64 {
    if z.is_finite() && z > 0.0 { z } else { 1.0 }
}

/// The one editable object selected (no anchors on their own), if it has a readable stack.
fn target(doc: &Document, sel: &vectorcraft_doc::Selection) -> Option<NodeId> {
    let [id] = sel.objects.as_slice() else { return None };
    if !sel.anchors.is_empty() || !sel.segments.is_empty() || !doc.is_editable(*id) {
        return None;
    }
    let n = doc.node(*id)?;
    if n.is_layer() {
        return None;
    }
    vectorcraft_modifiers::node_stack(n)?.ok().map(|_| *id)
}

/// `m`'s params with its bound fields replaced by what they resolved to in `out`.
fn resolved(m: &Modifier, out: Option<&vectorcraft_modifiers::Output>) -> ModifierKind {
    let Some(out) = out.filter(|_| !m.bind.is_empty()) else { return m.kind.clone() };
    let props: Map<String, Value> =
        out.bound.iter().filter(|b| b.modifier == m.id).filter_map(|b| Some((b.field.clone(), json!(b.value.filter(|v| v.is_finite())?)))).collect();
    let mut r = m.clone();
    if props.is_empty() || r.set_props(&props).is_err() {
        return m.kind.clone();
    }
    r.kind
}

/// The rows of object `id` whose handles show: those open in the panel (`open`) that have handles,
/// else the last enabled row with handles.
fn rows(doc: &Document, id: NodeId, open: Option<&[String]>) -> Vec<Row> {
    let Some(n) = doc.node(id) else { return Vec::new() };
    let Some(Ok(stack)) = vectorcraft_modifiers::node_stack(n) else { return Vec::new() };
    let candidates: Vec<(usize, &Modifier)> =
        stack.modifiers.iter().enumerate().filter(|(_, m)| m.enabled && vectorcraft_modifiers::handles::has_handles(&m.kind)).collect();
    let open: Vec<(usize, &Modifier)> = candidates.iter().filter(|(_, m)| open.is_some_and(|o| o.contains(&m.id))).copied().collect();
    let picked = if open.is_empty() { candidates.last().copied().into_iter().collect() } else { open };
    let out = if picked.iter().any(|(_, m)| !m.bind.is_empty()) { vectorcraft_modifiers::evaluate_node_in_doc(doc, n) } else { None };
    picked.into_iter().map(|(index, m)| Row { index, modifier: m.clone(), kind: resolved(m, out.as_deref()) }).collect()
}

fn is_bound(m: &Modifier, h: HandleKind) -> bool {
    h.fields(&m.kind).iter().any(|f| m.bind.contains_key(*f))
}

/// Object `id`'s handles at `zoom` for the rows `open`.
fn build(doc: &Document, id: NodeId, zoom: f64, open: Option<&[String]>) -> Option<Base> {
    let n = doc.node(id)?;
    let bounds = vectorcraft_modifiers::node_bounds_in_doc(doc, n);
    let rows = rows(doc, id, open);
    if rows.is_empty() {
        return None;
    }
    let f = Frame { node: n, bounds, zoom };
    let (mut handles, mut guides) = (Vec::new(), Vec::new());
    for (i, r) in rows.iter().enumerate() {
        guides.extend(vectorcraft_modifiers::handles::guides(&r.kind, &f));
        for s in vectorcraft_modifiers::handles::spots(&r.kind, &f) {
            handles.push((i, s, is_bound(&r.modifier, s.kind)));
        }
    }
    Some(Base { id, bounds, rows, handles, guides })
}

/// The bounding-box handles' places (corners and side middles) of the selection box `b`.
fn box_handles(b: &vectorcraft_doc::OrientedBox) -> Vec<Point> {
    let c = b.corners();
    let mut v = c.to_vec();
    for i in 0..4 {
        if let (Some(a), Some(e)) = (c.get(i), c.get((i + 1) % 4)) {
            v.push(a.midpoint(*e));
        }
    }
    v
}

/// The view's turn (degrees clockwise on screen), as the tools read it.
fn view_angle(view: &ViewInfo) -> f64 {
    view.screen.map(|s| (-s.right.y).atan2(s.right.x).to_degrees()).filter(|a| a.is_finite()).unwrap_or(0.0)
}

/// The cursor over a handle moving as `motion`, in a view turned `turn` degrees.
fn motion_cursor(motion: Motion, turn: f64) -> Cursor {
    match motion {
        Motion::Free => Cursor::Move,
        Motion::Turn => Cursor::Rotate,
        Motion::Along(d) => {
            let a = (d.y.atan2(d.x).to_degrees() + turn).rem_euclid(180.0);
            match ((a / 45.0).round() as i64).rem_euclid(4) {
                0 => Cursor::ResizeH,
                1 => Cursor::ResizeNwSe,
                2 => Cursor::ResizeV,
                _ => Cursor::ResizeNeSw,
            }
        }
    }
}

/// The handle under `p` in `base` within `reach`: the nearest; where several are as near, the
/// later row's, and within a row the one [`vectorcraft_modifiers::handles::spots`] lists first.
fn nearest(base: &Base, p: Point, reach: f64) -> Option<(usize, Spot, bool)> {
    let mut order: Vec<&(usize, Spot, bool)> = base.handles.iter().collect();
    // Stable: each row keeps its own order.
    order.sort_by_key(|h| std::cmp::Reverse(h.0));
    let mut best: Option<(f64, (usize, Spot, bool))> = None;
    for h in order {
        let d = h.1.at.distance(p);
        if d.is_finite() && d <= reach && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, *h));
        }
    }
    best.map(|(_, h)| h)
}

impl Session {
    /// The Appearance panel's open modifier rows of object `id` in the active document (their
    /// ids): their handles show on the canvas. An empty list: the last enabled row with handles
    /// shows them.
    pub fn picabia_set_handle_rows(&mut self, id: NodeId, rows: Vec<String>) {
        let Some(uid) = self.active().map(|st| st.uid) else { return };
        let rows: Vec<String> = rows.into_iter().take(vectorcraft_modifiers::MAX_STACK_LEN).collect();
        self.picabia.handles.rows = Some((uid, id, rows));
    }

    /// The drag in progress, if it is the active document's.
    fn live_drag(&self) -> Option<&HandleDrag> {
        let uid = self.active()?.uid;
        self.picabia.handles.drag.as_ref().filter(|d| d.uid == uid)
    }

    /// Drop a drag whose document was switched, whose object went or whose interaction something
    /// else ended; its preview, still open in its own (active) document, is rolled back. Whether
    /// there was one to drop.
    fn drop_stale_drag(&mut self) -> bool {
        let Some(d) = &self.picabia.handles.drag else { return false };
        let st = self.active();
        let same = st.is_some_and(|st| st.uid == d.uid);
        let node = same && st.is_some_and(|st| st.doc.node(d.id).is_some());
        let open = st.is_some_and(|st| st.interaction.is_some());
        let previewed = d.last.is_some();
        if same && node && (!previewed || open) {
            return false;
        }
        self.picabia.handles.drag = None;
        if same && previewed && open {
            // Its own preview: put the document back as the drag found it.
            let _ = self.apply_actions(vec![Action::Cancel]);
        }
        true
    }

    /// Whether modifier handles show now: the Selection tool (not lent for a Cmd-drag) is idle or
    /// dragging one.
    fn handles_shown(&self) -> bool {
        let dragging = self.live_drag().is_some();
        self.tool.id() == "selection" && self.lender.is_none() && (dragging || !self.tool.busy())
    }

    /// The selected object's handles at `zoom` (built once per document version, selection, zoom
    /// and open rows).
    fn handle_base(&self, zoom: f64) -> Option<Arc<Base>> {
        if !self.handles_shown() {
            return None;
        }
        let st = self.active()?;
        let id = match self.live_drag() {
            Some(d) => d.id,
            None => target(&st.doc, &st.selection)?,
        };
        let open = self.picabia.handles.rows.as_ref().filter(|(u, o, _)| *u == st.uid && *o == id).map(|(.., r)| r.clone());
        let key = BaseKey { uid: st.uid, revision: st.revision, zoom: zoom.to_bits(), id, open };
        let mut cache = self.picabia.handles.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((k, b)) = cache.as_ref()
            && *k == key
        {
            return b.clone();
        }
        #[cfg(test)]
        self.picabia.handles.builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = build(&st.doc, id, zoom, key.open.as_deref()).map(Arc::new);
        *cache = Some((key, base.clone()));
        base
    }

    /// The selected object's modifier handles as the canvas draws them now, at `view`'s zoom.
    pub fn picabia_mod_handles(&self, view: ViewInfo) -> Option<ModHandlesView> {
        let base = self.handle_base(sane_zoom(view.zoom))?;
        let uid = self.active()?.uid;
        let hs = &self.picabia.handles;
        let drag = self.live_drag();
        let hot = |m: &str, h: HandleKind| match (drag, &hs.hover) {
            (Some(d), _) => d.modifier == m && d.handle == h,
            (None, Some((u, id, hm, hh))) => *u == uid && *id == base.id && hm == m && *hh == h,
            _ => false,
        };
        let mut out = ModHandlesView { guides: base.guides.clone(), ..ModHandlesView::default() };
        for (row, s, bound) in &base.handles {
            let Some(r) = base.rows.get(*row) else { continue };
            let hot = hot(&r.modifier.id, s.kind);
            if hot && out.readout.is_none() {
                let text = if *bound {
                    Some("Bound to a parameter: change it in the Appearance panel".to_string())
                } else {
                    vectorcraft_modifiers::handles::label(&r.kind, s.kind)
                };
                out.readout = text.map(|t| (s.at, t));
            }
            out.handles.push(ModHandleView { modifier: r.modifier.id.clone(), index: r.index, kind: s.kind, at: s.at, bound: *bound, hot });
        }
        Some(out)
    }

    /// The handle under `p` and the kept handles it is one of; none where a bounding-box handle is.
    fn handle_at(&mut self, p: Point, view: ViewInfo) -> Option<((usize, Spot, bool), Arc<Base>)> {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        let zoom = sane_zoom(view.zoom);
        let base = self.handle_base(zoom)?;
        let hit = nearest(&base, p, HIT_PX / zoom)?;
        if view.show_bbox {
            let bx = self.with_tool_cx(view, |_, cx| vectorcraft_tools::select::selection_box(cx));
            if bx.is_some_and(|b| box_handles(&b).iter().any(|q| q.distance(p) <= BOX_HANDLE_PX / zoom)) {
                return None;
            }
        }
        Some((hit, base))
    }

    /// A pointer event for the modifier handles, before the tool sees it: `Some` when they take it.
    pub(crate) fn picabia_handles_pointer(&mut self, ev: &PointerEvent, view: ViewInfo) -> Option<Result<Vec<UiRequest>>> {
        if self.picabia.handles.drag.is_some() {
            if self.drop_stale_drag() {
                // The gesture is gone: the press (or the rest of the drag) is nobody's.
                return Some(Ok(vec![]));
            }
            return match ev.kind {
                PointerKind::Drag => Some(self.handle_drag(ev.pos, ev.mods)),
                PointerKind::Up => Some(self.handle_drag(ev.pos, ev.mods).and_then(|mut ui| {
                    ui.extend(self.handle_finish()?);
                    Ok(ui)
                })),
                // A press without a release in between: the drag ends where it is.
                PointerKind::Down | PointerKind::DoubleClick => Some(self.handle_finish()),
                PointerKind::Move => Some(Ok(vec![])),
            };
        }
        let plain = !(ev.mods.alt || ev.mods.cmd || ev.mods.ctrl);
        let uid = self.active().map(|st| st.uid);
        match ev.kind {
            PointerKind::Move => {
                let h = if plain { self.handle_at(ev.pos, view) } else { None };
                self.picabia.handles.hover = h.and_then(|((row, s, _), b)| Some((uid?, b.id, b.rows.get(row)?.modifier.id.clone(), s.kind)));
                None
            }
            PointerKind::Down if plain => {
                let ((row, s, bound), base) = self.handle_at(ev.pos, view)?;
                let r = base.rows.get(row)?;
                self.picabia.handles.hover = Some((uid?, base.id, r.modifier.id.clone(), s.kind));
                if bound {
                    return Some(Ok(vec![UiRequest::Status(format!(
                        "{} is bound to a parameter or an expression: change it in the Appearance panel",
                        s.kind.fields(&r.kind).join(" and ")
                    ))]));
                }
                let params = r.kind.params();
                let from = s.kind.fields(&r.kind).iter().filter_map(|f| Some(((*f).to_string(), params.get(*f)?.clone()))).collect();
                self.picabia.handles.drag = Some(HandleDrag {
                    uid: uid?,
                    id: base.id,
                    modifier: r.modifier.id.clone(),
                    handle: s.kind,
                    bound: r.modifier.bind.keys().cloned().collect(),
                    kind: r.kind.clone(),
                    bounds: base.bounds,
                    zoom: sane_zoom(view.zoom),
                    grab: s.at - ev.pos,
                    start: s.at,
                    from,
                    last: None,
                });
                Some(Ok(vec![]))
            }
            _ => None,
        }
    }

    /// Preview the params the pointer at `p` gives, when they change; the first change opens the
    /// undo step.
    fn handle_drag(&mut self, p: Point, mods: Mods) -> Result<Vec<UiRequest>> {
        let Some(d) = &self.picabia.handles.drag else { return Ok(vec![]) };
        let Some(n) = self.active().and_then(|st| st.doc.node(d.id)) else { return Ok(vec![]) };
        let f = Frame { node: n, bounds: d.bounds, zoom: d.zoom };
        let Some(mut props) = vectorcraft_modifiers::handles::drag(&d.kind, d.handle, &f, p + d.grab, d.start, mods.shift, d.last.as_ref()) else {
            return Ok(vec![]);
        };
        props.retain(|k, _| !d.bound.contains(k));
        if props.is_empty() || d.last.as_ref().unwrap_or(&d.from) == &props {
            return Ok(vec![]);
        }
        let mut acts = Vec::new();
        if d.last.is_none() {
            acts.push(Action::Begin("Set Modifier".into()));
        }
        acts.push(Action::Preview("modifier.set".into(), json!({"ids": [d.id.0], "modifier": d.modifier, "props": props})));
        if let Some(d) = &mut self.picabia.handles.drag {
            d.last = Some(props);
        }
        self.apply_actions(acts)
    }

    /// The drag is over: one undo step, or none when it ends where it began.
    fn handle_finish(&mut self) -> Result<Vec<UiRequest>> {
        let Some(d) = self.picabia.handles.drag.take() else { return Ok(vec![]) };
        match d.last {
            Some(v) if v == d.from => self.apply_actions(vec![Action::Cancel]),
            Some(_) => self.apply_actions(vec![Action::Commit]),
            None => Ok(vec![]),
        }
    }

    /// Escape during a handle drag puts the params back; `Some` when the handles took the key.
    pub(crate) fn picabia_handles_key(&mut self, key: ToolKey) -> Option<Result<Vec<UiRequest>>> {
        if key != ToolKey::Escape || self.picabia.handles.drag.is_none() || self.drop_stale_drag() {
            return None;
        }
        let d = self.picabia.handles.drag.take()?;
        Some(if d.last.is_some() { self.apply_actions(vec![Action::Cancel]) } else { Ok(vec![]) })
    }

    /// Whether the handles take `key` ahead of the shortcuts (Escape during a drag).
    pub(crate) fn picabia_handles_claim(&self, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.live_drag().is_some()
    }

    /// Whether a handle drag of the active document is in progress.
    pub(crate) fn picabia_handles_busy(&self) -> bool {
        self.live_drag().is_some()
    }

    /// The cursor over a modifier handle at `p` (or during its drag), if there is one.
    pub(crate) fn picabia_handles_cursor(&mut self, p: Point, mods: Mods, view: ViewInfo) -> Option<Cursor> {
        let turn = view_angle(&view);
        if let Some(d) = self.live_drag() {
            return Some(motion_cursor(d.handle.motion(&d.kind), turn));
        }
        if mods.alt || mods.cmd || mods.ctrl {
            return None;
        }
        let ((row, s, bound), base) = self.handle_at(p, view)?;
        let kind = &base.rows.get(row)?.kind;
        Some(if bound { Cursor::NotAllowed } else { motion_cursor(s.kind.motion(kind), turn) })
    }

    /// `modifier.handles`: the handles the canvas shows for the selection now (at the last view's
    /// zoom).
    pub(crate) fn picabia_handles_json(&self) -> Value {
        let Some(v) = self.picabia_mod_handles(self.last_view) else { return json!({"handles": []}) };
        let id = self.handle_base(sane_zoom(self.last_view.zoom)).map(|b| b.id.0);
        let hs: Vec<Value> = v
            .handles
            .iter()
            .map(|h| json!({"modifier": h.modifier, "index": h.index, "handle": h.kind.id(), "x": h.at.x, "y": h.at.y, "bound": h.bound}))
            .collect();
        json!({"id": id, "handles": hs})
    }

    /// How many times the handles were built (the cache's test).
    #[cfg(test)]
    pub(crate) fn picabia_handle_builds(&self) -> usize {
        self.picabia.handles.builds.load(std::sync::atomic::Ordering::Relaxed)
    }
}
