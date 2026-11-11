//! PICABIA: an object with a modifier stack draws as its copies.
//!
//! The object becomes "art" the way VectorCraft's art-replacing effects do (Crop Marks, Vector
//! Halftone): [`art`] is a group that takes the object's place and identity, holding one copy of
//! the object per modifier instance, its geometry transformed by the instance transform
//! (`Node::transform`, strokes, effects, corners and patterns scaling with it, as
//! `modifier.apply` bakes it). The renderer draws it through `fx::has_object_fx` and the `fx_art`
//! chain; bounds, culling and dirty regions measure it in `fx::cull_bounds_in`; every vector
//! exporter writes it through `vectorcraft_effects::picabia` (registered from the engine); hit
//! testing tries the object under each instance transform through `vectorcraft_doc::hit` hooks.
//!
//! # Evaluation order
//! Geometry rows (`resample`, `trim`, `tidy`) above every copy row reshape the object before it
//! is copied, and those below one reshape each copy after its transform
//! (`vectorcraft_modifiers::Shaper`); an object whose only rows are geometry rows draws as a group
//! of one reshaped copy, and is clicked as drawn (its art), not by transforms.
//! Copies are made from the object's geometry first; each copy then draws with the object's
//! fills, strokes and effects (geometry effects and raster effects such as a drop shadow apply to
//! each copy). The object's transparency (opacity, blending mode, isolation, opacity mask) goes
//! on the group: the copies are composited together, once, and never knock each other out. No
//! copy carries the stack, so the art never recurses; members of a copied group keep their own
//! stacks, within one budget.
//!
//! # Stencils
//! An object whose stack ends in a stencil row draws, once its sheet is computed (fresh, or the
//! latest while it is computed again), as the sheet's paper or cloth preview in place of its design
//! ([`vectorcraft_stencil::store::drawn`] / `fills`); before that, as its copies (or itself). Its
//! bounds and clicks are the sheet's from the start ([`vectorcraft_stencil::store::frame`]).
//!
//! `color` rows recolour each copy after its transform and the geometry rows ([`colour`]): fills
//! and strokes shift in OKLCH by the copy's shifts, in one place, so the canvas, every export,
//! `modifier.apply` and Package agree. A stack whose only rows are `color` rows draws as one
//! recoloured copy (it makes a change).
//!
//! An object whose stack makes no copies (no active row, one unmoved copy, a copy-to-shape target
//! that is missing, or more cloned objects than [`MAX_ART_NODES`]) has no art: it draws exactly as
//! upstream draws it, as [`plain`] (itself without the stack).
//!
//! # Cost when nothing has a stack
//! [`has_stack`] is an attribute lookup. [`subtree_flags`] says whether a container holds stacks
//! (or copy-to-shape sources), kept per container by identity, so after an edit only the changed
//! containers are looked at again: hit testing, dirty regions and caches ask it instead of walking
//! the document.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use vectorcraft_doc::{Document, Node, Scaling, SymbolBoxes};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_modifiers::Output;

/// The Color row's recolouring of a copy.
#[path = "picabia_colour.rs"]
pub mod colour;

/// The most nodes the kept arts hold together, their objects included (the least recently used go
/// first): what a closed document's art can keep alive is bounded by it.
const MAX_CACHED_NODES: usize = 100_000;

/// The most path anchors the kept arts hold together (the least recently used go first): geometry
/// rows can put many anchors in few nodes, so the cache is weighed by both.
const MAX_CACHED_ANCHORS: usize = 4 * vectorcraft_modifiers::MAX_GEOMETRY_ANCHORS;

/// The most copies an object's art may draw with the copies its members' own stacks make inside
/// each of them (the product across nesting levels); past this the members draw once in each copy.
pub const MAX_NESTED_COPIES: usize = vectorcraft_modifiers::MAX_INSTANCES_PER_NODE;

/// The most nodes one object's art may clone (copies × nodes in the object, as
/// `modifier.apply`'s cap); past this the object draws once, plainly, with a warning.
pub const MAX_ART_NODES: usize = 100_000;

/// The most containers whose [`Flags`] are kept (the dead ones go first past it, then all, and
/// are found again).
const MAX_FLAGGED: usize = 50_000;

/// An art kept: the address of the object it was made for, the evaluation it was made from (the
/// modifiers crate gives a new one when the stack, the bounds or a copy-to-shape target changes),
/// the object (compared outside the lock), the art (`None`: over [`MAX_ART_NODES`]), its size and
/// what the geometry rows said while it was made.
struct Kept {
    at: usize,
    out: Arc<Output>,
    node: Node,
    art: Option<Arc<Node>>,
    nodes: usize,
    /// The path anchors the art and the object hold.
    anchors: usize,
    notes: Vec<String>,
}

/// The path anchors in `n` and everything in it.
fn anchor_count(n: &Node) -> usize {
    fn walk(n: &Node, depth: usize) -> usize {
        if depth > vectorcraft_doc::MAX_DEPTH {
            return 0;
        }
        let own = n.path_data().map_or(0, vectorcraft_geom::PathData::anchor_count);
        n.children().into_iter().flatten().fold(own, |a, c| a.saturating_add(walk(c, depth + 1)))
    }
    walk(n, 0)
}

static CACHE: Mutex<Vec<Arc<Kept>>> = Mutex::new(Vec::new());

/// What a container holds below it and in itself: active stacks, and copy-to-shape sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub stacks: bool,
    pub targets: bool,
}

impl Flags {
    fn or(self, o: Flags) -> Flags {
        Flags { stacks: self.stacks || o.stacks, targets: self.targets || o.targets }
    }
}

/// What a container holds, in all and below itself, kept by its address. The entry holds the
/// container weakly: nothing it holds (members, images) is kept alive, and while the entry is kept
/// no other node can take the address.
type Kept2 = (Weak<Node>, Flags, Flags);

static FLAGS: Mutex<Option<HashMap<usize, Kept2>>> = Mutex::new(None);

fn kept_flags(key: usize) -> Option<(Flags, Flags)> {
    let kept = FLAGS.lock().unwrap_or_else(PoisonError::into_inner);
    kept.as_ref().and_then(|m| m.get(&key)).map(|(_, all, below)| (*all, *below))
}

/// Whether `n` has a stack with an active row (an attribute lookup and a cached parse: cheap
/// enough for every object on every frame). It may still make no copies ([`art`]).
pub fn has_stack(n: &Node) -> bool {
    vectorcraft_modifiers::has_modifiers(n) && matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.is_active())
}

/// Whether `n`'s stack copies onto another object or has bound fields: its art then changes with
/// that object or with the document's parameters, so caches kept by `n`'s identity alone don't
/// hold it.
pub fn has_targets(n: &Node) -> bool {
    vectorcraft_modifiers::has_modifiers(n)
        && matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.targets().next().is_some() || s.has_bindings())
}

/// `targets` also stands for a stencil: its drawing changes when its sheet arrives, not only with
/// the object.
fn own_flags(n: &Node) -> Flags {
    Flags { stacks: has_stack(n), targets: has_targets(n) || vectorcraft_stencil::store::is_stencil(n) }
}

/// What `a` and everything in it hold ([`Flags`]); kept per container, so it costs what changed.
pub fn subtree_flags(a: &Arc<Node>) -> Flags {
    subtree_flags_at(a, 0)
}

fn subtree_flags_at(a: &Arc<Node>, depth: usize) -> Flags {
    let own = own_flags(a);
    let Some(children) = a.children() else { return own };
    // A kept entry holds its container: the address is this container's.
    if let Some((all, _)) = kept_flags(Arc::as_ptr(a) as usize) {
        return all;
    }
    let mut below = Flags::default();
    if depth <= vectorcraft_doc::MAX_DEPTH {
        for c in children {
            below = below.or(subtree_flags_at(c, depth + 1));
        }
    }
    let all = own.or(below);
    let mut kept = FLAGS.lock().unwrap_or_else(PoisonError::into_inner);
    let map = kept.get_or_insert_with(HashMap::new);
    if map.len() >= MAX_FLAGGED {
        map.retain(|_, (w, ..)| w.strong_count() > 0);
        if map.len() >= MAX_FLAGGED {
            map.clear();
        }
    }
    map.insert(Arc::as_ptr(a) as usize, (Arc::downgrade(a), all, below));
    all
}

/// What every layer of `doc` holds.
pub fn doc_flags(doc: &Document) -> Flags {
    doc.layers.iter().fold(Flags::default(), |f, l| f.or(subtree_flags(l)))
}

/// Whether something below `n` (not `n` itself) has an active stack: a click outside `n`'s own
/// reach may still land on its members' copies. A kept-flag lookup for a container the document
/// was flagged with; otherwise its members' flags.
pub fn holds_copies(n: &Node) -> bool {
    let Some(children) = n.children() else { return false };
    if let Some((_, below)) = kept_flags(std::ptr::from_ref(n) as usize) {
        return below.stacks;
    }
    children.iter().any(|c| subtree_flags(c).stacks)
}

/// [`holds_copies`] for clicks in `symbols`' document: nothing to look at when the document has no
/// stacks (its layers' kept flags say so; looking flags every container for the lookups after).
fn hit_holds(symbols: &SymbolBoxes, n: &Node) -> bool {
    has_stacks(symbols.document()) && holds_copies_kept(n)
}

fn holds_copies_kept(n: &Node) -> bool {
    let Some(children) = n.children() else { return false };
    if let Some((_, below)) = kept_flags(std::ptr::from_ref(n) as usize) {
        return below.stacks;
    }
    children.iter().any(|c| subtree_flags(c).stacks)
}

/// Whether `a` or something in it copies onto another object or is a stencil: caches kept by
/// `a`'s identity alone don't hold its drawing.
pub fn holds_targets(a: &Arc<Node>) -> bool {
    if a.children().is_none() {
        return has_targets(a) || vectorcraft_stencil::store::is_stencil(a);
    }
    subtree_flags(a).targets
}

/// The evaluation of `n`'s stack when it makes copies, or `None` when it draws once as itself.
fn copies(doc: &Document, n: &Node) -> Option<Arc<Output>> {
    if !has_stack(n) {
        return None;
    }
    let out = vectorcraft_modifiers::evaluate_node_in_doc(doc, n)?;
    // One unmoved copy is the object itself, unless geometry or colour rows change it.
    let unmoved = matches!(out.instances.as_slice(), [one] if one.transform == Affine::IDENTITY) && !out.has_geometry() && !out.has_colour();
    (!out.instances.is_empty() && !unmoved).then_some(out)
}

/// Whether `n` draws as copies in `doc` (it has an active stack that makes copies within
/// [`MAX_ART_NODES`]), found without building the copies.
pub fn draws_copies(doc: &Document, n: &Node) -> bool {
    has_stack(n) && (draws_sheet(doc, n) || copies(doc, n).is_some_and(|out| node_count(n).saturating_mul(out.instances.len()) <= MAX_ART_NODES))
}

/// Whether `n` is a stencil drawn as its sheet (one was computed, and it has a size to cut: what
/// [`sheet_art`] needs).
pub fn draws_sheet(doc: &Document, n: &Node) -> bool {
    vectorcraft_stencil::store::is_stencil(n) && vectorcraft_stencil::store::drawn(doc, n).is_some()
}

/// How many stencil arts are kept.
const KEPT_SHEETS: usize = 16;

/// `(object address, sheet, object, art)`: a stencil's preview kept for its object and sheet.
type KeptSheet = (usize, Arc<vectorcraft_stencil::store::Sheet>, Node, Arc<Node>);

static SHEETS: Mutex<Vec<Arc<KeptSheet>>> = Mutex::new(Vec::new());

/// A stencil object's sheet preview (see the module docs): a group in its place holding the fills,
/// with its transparency; `None` before a sheet is computed.
fn sheet_art(doc: &Document, n: &Node) -> Option<Arc<Node>> {
    let d = vectorcraft_stencil::store::drawn(doc, n)?;
    let at = std::ptr::from_ref(n) as usize;
    let candidate = {
        let kept = SHEETS.lock().unwrap_or_else(PoisonError::into_inner);
        kept.iter().find(|k| k.0 == at && Arc::ptr_eq(&k.1, &d.sheet)).cloned()
    };
    if let Some(k) = candidate.filter(|k| k.2 == *n) {
        return Some(k.3.clone());
    }
    let mut g = Node::group(n.id, vec![]);
    (g.visible, g.locked) = (n.visible, n.locked);
    g.name = n.name.clone();
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    g.extras_mut().mask = n.extras().mask.clone();
    let fills = vectorcraft_stencil::store::fills(&d)
        .into_iter()
        .map(|f| {
            let [r, gr, b] = f.rgb.map(|c| f32::from(c) / 255.0);
            let paint = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(r, gr, b));
            let mut p = Node::path(
                n.id,
                vectorcraft_geom::PathData::from_bezpath(&f.path),
                vectorcraft_doc::Appearance::basic(paint, vectorcraft_color::Paint::None, 0.0),
            );
            if let vectorcraft_doc::NodeKind::Path { rule, .. } = &mut p.kind {
                *rule = if f.even_odd { vectorcraft_geom::FillRule::EvenOdd } else { vectorcraft_geom::FillRule::NonZero };
            }
            Arc::new(p)
        })
        .collect();
    if let Some(ch) = g.children_mut() {
        *ch = fills;
    }
    let art = Arc::new(g);
    let mut kept = SHEETS.lock().unwrap_or_else(PoisonError::into_inner);
    kept.retain(|k| k.0 != at);
    kept.insert(0, Arc::new((at, d.sheet.clone(), n.clone(), art.clone())));
    kept.truncate(KEPT_SHEETS);
    Some(art)
}

/// A stencil object's sheet rectangle (bounds and clicks), and whether the sheet is all it draws.
/// Known from its params before a sheet is computed (then its design or copies draw too, `false`).
/// With a result: where it is now, and where the result drawn lies (a stale one is drawn where it
/// was computed until the new one lands, so a moved stencil's old place repaints and clicks).
fn sheet_box(doc: &Document, n: &Node) -> Option<(vectorcraft_geom::Rect, bool)> {
    if !vectorcraft_stencil::store::is_stencil(n) {
        return None;
    }
    let frame = vectorcraft_stencil::store::frame(doc, n)?;
    match vectorcraft_stencil::store::lookup(doc, n) {
        Some((sheet, _)) => Some((frame.sheet.union(sheet.sheet), true)),
        None => Some((frame.sheet, false)),
    }
}

/// Whether `n`'s members draw once in each of its copies, as [`make`] decides: their own copies
/// over the budget, or the anchors their geometry rows make, across all of `n`'s copies, over the
/// geometry cap (the cap is per object; nesting would multiply it).
fn strips_nested(doc: &Document, n: &Node, instances: usize) -> bool {
    let nested = nested_copies(doc, n, 0);
    if nested == 0 {
        return false;
    }
    instances.saturating_mul(nested) > MAX_NESTED_COPIES
        || instances.saturating_mul(nested_anchors(doc, n, 0)) > vectorcraft_modifiers::MAX_GEOMETRY_ANCHORS
}

/// Why `n` draws once although its stack makes copies, for the panel's warnings: too many nodes.
pub fn too_big(doc: &Document, n: &Node) -> Option<String> {
    let out = copies(doc, n)?;
    let wanted = node_count(n).saturating_mul(out.instances.len());
    (wanted > MAX_ART_NODES).then(|| format!("{wanted} objects to draw (at most {MAX_ART_NODES}): drawn once"))
}

/// What a copy's transform scales besides its geometry: everything, as a copy drawn bigger.
pub fn scaling() -> Scaling {
    Scaling {
        strokes: true,
        effects: Some(vectorcraft_effects::scale_effect),
        keep_type_strokes: false,
        keep_corners: false,
        patterns: true,
        keep_fit_when_flat: false,
    }
}

fn node_count(n: &Node) -> usize {
    n.count()
}

/// `n`'s art in `doc`: the group of its copies (see the module docs), or `None` when it draws as
/// itself. Kept while `n` and its evaluation stay the same.
pub fn art(doc: &Document, n: &Node) -> Option<Arc<Node>> {
    if vectorcraft_stencil::store::is_stencil(n)
        && let Some(a) = sheet_art(doc, n)
    {
        return Some(a);
    }
    copies_art(doc, n)
}

/// The group of `n`'s copies (geometry rows applied), never a stencil's sheet: what a stencil cuts
/// is gathered from it. `None` when `n` draws once as itself.
pub fn copies_art(doc: &Document, n: &Node) -> Option<Arc<Node>> {
    kept(doc, n)?.art.clone()
}

/// What `n`'s geometry rows said while its copies were made (a row skipped over the anchor cap,
/// type passed over…), for the panel. Empty when it has no copies.
pub fn notes(doc: &Document, n: &Node) -> Vec<String> {
    kept(doc, n).map(|k| k.notes.clone()).unwrap_or_default()
}

/// `n`'s kept copies, made when they aren't kept (see [`copies_art`]).
fn kept(doc: &Document, n: &Node) -> Option<Arc<Kept>> {
    Some(kept_for(doc, n, copies(doc, n)?))
}

/// [`kept`] for `n`'s evaluation `out` (from [`copies`]).
fn kept_for(doc: &Document, n: &Node, out: Arc<Output>) -> Arc<Kept> {
    let at = std::ptr::from_ref(n) as usize;
    let candidate = {
        let cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        cache.iter().find(|k| k.at == at && Arc::ptr_eq(&k.out, &out)).cloned()
    };
    // The object compared outside the lock (its address may have been reused).
    if let Some(k) = candidate.filter(|k| k.node == *n) {
        return k;
    }
    let nodes = node_count(n).saturating_mul(out.instances.len());
    let (art, notes) = if nodes <= MAX_ART_NODES {
        let (art, notes) = make(doc, n, &out);
        (Some(Arc::new(art)), notes)
    } else {
        (None, Vec::new())
    };
    let held = node_count(n).saturating_add(if art.is_some() { nodes } else { 0 });
    let anchors = anchor_count(n).saturating_add(art.as_deref().map_or(0, anchor_count));
    let kept = Arc::new(Kept { at, out, node: n.clone(), art, nodes: held, anchors, notes });
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|k| !(k.at == at && k.node == *n));
    cache.insert(0, kept.clone());
    let (mut total, mut total_anchors) = (0usize, 0usize);
    cache.retain(|k| {
        total = total.saturating_add(k.nodes);
        total_anchors = total_anchors.saturating_add(k.anchors);
        total <= MAX_CACHED_NODES && total_anchors <= MAX_CACHED_ANCHORS
    });
    kept
}

/// [`art`] as an owned node, for the renderer's art chain and the exporters' bake.
pub fn art_node(doc: &Document, n: &Node) -> Option<Node> {
    art(doc, n).map(Arc::unwrap_or_clone)
}

/// `n` without its stack: what an object whose stack makes no copies draws as (exactly as
/// upstream draws it).
pub fn plain(n: &Node) -> Node {
    let mut m = n.clone();
    vectorcraft_modifiers::set_node_source(&mut m, None);
    m
}

/// `art` without a stack of its own (an object's art is drawn as it is, never through its stack
/// again).
pub fn unstacked(art: Node) -> Node {
    if vectorcraft_modifiers::has_modifiers(&art) { plain(&art) } else { art }
}

/// How many [`plain_arc`]s are kept.
const KEPT_PLAIN: usize = 256;

/// `(source, plain)`, the source held weakly; entries whose source is gone are dropped first.
static PLAIN: Mutex<Vec<(Weak<Node>, Arc<Node>)>> = Mutex::new(Vec::new());

/// For the renderer: `a` without its stack when it has an active one that makes no copies, kept
/// by `a`'s identity (so the renderer's caches keyed by identity keep working); `None` otherwise
/// (draw `a` itself).
pub fn plain_arc(doc: &Document, a: &Arc<Node>) -> Option<Arc<Node>> {
    if !has_stack(a) || draws_copies(doc, a) {
        return None;
    }
    let mut kept = PLAIN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = kept.iter().position(|(s, _)| std::ptr::eq(s.as_ptr(), Arc::as_ptr(a))) {
        let e = kept.remove(at);
        let p = e.1.clone();
        kept.insert(0, e);
        return Some(p);
    }
    let p = Arc::new(plain(a));
    kept.retain(|(s, _)| s.strong_count() > 0);
    kept.insert(0, (Arc::downgrade(a), p.clone()));
    kept.truncate(KEPT_PLAIN);
    Some(p)
}

/// What bounds measure for `n`: its copies' group, or itself without the stack when the stack
/// makes no copies; `None` without an active stack (measure `n`).
pub fn bounds_art(doc: &Document, n: &Node) -> Option<Arc<Node>> {
    if !has_stack(n) {
        return None;
    }
    // A stencil reaches as far as its sheet, before and after it is computed; before, as far as
    // its design or copies too (a tile output's grid draws past the tile).
    if let Some((r, only)) = sheet_box(doc, n) {
        let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::kurbo::Shape::to_path(&r, 0.1));
        let solid = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
        let sheet = Arc::new(Node::path(n.id, path, vectorcraft_doc::Appearance::basic(solid, vectorcraft_color::Paint::None, 0.0)));
        if only {
            return Some(sheet);
        }
        let design = art(doc, n).unwrap_or_else(|| Arc::new(plain(n)));
        return Some(Arc::new(Node::group(n.id, vec![sheet, design])));
    }
    Some(art(doc, n).unwrap_or_else(|| Arc::new(plain(n))))
}

/// The group of `n`'s copies for `out`, its geometry rows run on the object before copying and on
/// each copy after its transform; and what those rows said.
fn make(doc: &Document, n: &Node, out: &Output) -> (Node, Vec<String>) {
    let mut object = n.clone();
    let mut g = Node::group(n.id, vec![]);
    (g.visible, g.locked) = (n.visible, n.locked);
    g.name = object.name.take();
    g.opacity = std::mem::replace(&mut object.opacity, 1.0);
    g.blend = std::mem::take(&mut object.blend);
    g.isolate = std::mem::take(&mut object.isolate);
    g.extras_mut().mask = object.extras_mut().mask.take();
    g.extras_mut().attrs = object.extras_mut().attrs.take();
    // The copies never knock each other out (members inside each copy still do, as the object says).
    g.knockout = vectorcraft_doc::Knockout::Off;
    // The art is drawn and written as it is: it carries no stack (a written copy that did would
    // be copied again when the file is read back).
    vectorcraft_modifiers::set_node_source(&mut g, None);
    // Members with stacks of their own copy inside every copy: within one budget.
    let mut stripped = None;
    if strips_nested(doc, &object, out.instances.len()) {
        if nested_anchors(doc, &object, 0) > 0 {
            stripped = Some(format!(
                "members' geometry rows would make more than {} anchors in all the copies: the members are drawn without their modifiers",
                vectorcraft_modifiers::MAX_GEOMETRY_ANCHORS
            ));
        }
        strip_nested(&mut object, 0);
    }
    let sc = scaling();
    let mut shaper = vectorcraft_modifiers::Shaper::new(out);
    shaper.notes.extend(stripped);
    shaper.source(&mut object);
    let recolourer = out.has_colour().then(|| colour::Recolourer::new(doc));
    if recolourer.is_some() && colour::has_pattern(&object) {
        shaper.notes.push(colour::PATTERN_NOTE.to_string());
    }
    let copies = out
        .instances
        .iter()
        .enumerate()
        .filter(|(_, i)| is_finite(i.transform))
        .map(|(k, i)| {
            let mut c = object.clone();
            if i.transform != Affine::IDENTITY {
                c.transform(i.transform, sc);
            }
            // Which copy: a deform and a displace ramp across them (a deform bends each in its own frame).
            shaper.copy_at(&mut c, k);
            // Then its own colours (`color` rows), after its transform and the geometry rows.
            if let Some(r) = &recolourer {
                colour::recolour(r, &mut c, out.colour(k));
            }
            Arc::new(c)
        })
        .collect();
    if let Some(ch) = g.children_mut() {
        *ch = copies;
    }
    (g, shaper.notes)
}

/// How many copies the members of `n` (not `n` itself) draw through their own stacks, inside one
/// copy of `n`: each member's instances times what its own members draw (doc-aware, so
/// copy-to-shape members count their copies).
fn nested_copies(doc: &Document, n: &Node, depth: usize) -> usize {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return 0;
    }
    let mut total = 0usize;
    for c in n.children().into_iter().flatten() {
        if has_stack(c) {
            let k = copies(doc, c).map_or(1, |o| o.instances.len());
            total = total.saturating_add(k.saturating_mul(1 + nested_copies(doc, c, depth + 1)));
        } else {
            total = total.saturating_add(nested_copies(doc, c, depth + 1));
        }
    }
    total
}

/// The anchors the geometry rows of `n`'s members (not `n` itself) put in one copy of `n`: each
/// reshaped member's art, times the copies of the members above it that copy it. Building a
/// member's art is bounded by its own cap.
fn nested_anchors(doc: &Document, n: &Node, depth: usize) -> usize {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return 0;
    }
    let mut total = 0usize;
    for c in n.children().into_iter().flatten() {
        let here = match has_stack(c).then(|| copies(doc, c)).flatten() {
            Some(o) if o.has_geometry() => copies_art(doc, c).map_or(0, |a| anchor_count(&a)),
            Some(o) => o.instances.len().saturating_mul(nested_anchors(doc, c, depth + 1)),
            None => nested_anchors(doc, c, depth + 1),
        };
        total = total.saturating_add(here);
    }
    total
}

/// `n`'s members without their stacks.
fn strip_nested(n: &mut Node, depth: usize) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        let c = Arc::make_mut(c);
        vectorcraft_modifiers::set_node_source(c, None);
        strip_nested(c, depth + 1);
    }
}

fn is_finite(a: Affine) -> bool {
    a.as_coeffs().iter().all(|c| c.is_finite())
}

/// The hooks `vectorcraft_doc::hit` uses: an object is clicked through its copies, as itself
/// under each instance transform (no art is built for a click), or, when geometry rows reshape the
/// copies, as its art.
pub fn hit_hooks() -> vectorcraft_doc::hit::CopyHooks {
    vectorcraft_doc::hit::CopyHooks {
        has: has_stack,
        holds: hit_holds,
        transforms: hit_transforms,
        art: hit_art,
        shaped: hit_shaped,
        sheet: hit_sheet,
    }
}

/// `n`'s copies when geometry rows reshape them (a transform of `n` isn't what is drawn); for a
/// stencil, tried after its sheet when the design draws too.
fn hit_shaped(symbols: &SymbolBoxes, n: &Node) -> Option<Arc<Node>> {
    let doc = symbols.document();
    // Plain copies are clicked by their transforms: no art is built for them.
    let out = copies(doc, n).filter(|o| o.has_geometry())?;
    kept_for(doc, n, out).art.clone()
}

/// The transforms of `n`'s copies, the front one last, and whether its members draw once in each
/// (as [`make`] decides); none when it draws once. No copy is built.
fn hit_transforms(symbols: &SymbolBoxes, n: &Node) -> Option<(Arc<[Affine]>, bool)> {
    let doc = symbols.document();
    if !draws_copies(doc, n) {
        return None;
    }
    let out = copies(doc, n)?;
    let strip = strips_nested(doc, n, out.instances.len());
    Some((out.instances.iter().map(|i| i.transform).filter(|t| is_finite(*t)).collect(), strip))
}

/// A stencil object's sheet, clicked as the object.
fn hit_sheet(symbols: &SymbolBoxes, n: &Node) -> Option<(vectorcraft_geom::Rect, bool)> {
    sheet_box(symbols.document(), n)
}

/// `n`'s art, for marquees (each copy touched as `n`).
fn hit_art(symbols: &SymbolBoxes, n: &Node) -> Option<Arc<Node>> {
    art(symbols.document(), n)
}

/// Whether any object of `doc` has an active stack (kept per container: costs what changed).
pub fn has_stacks(doc: &Document) -> bool {
    doc_flags(doc).stacks
}

/// `doc` with every object that has modifier copies replaced by them, as plain objects with ids of
/// their own and no stack (what `modifier.apply` makes, without the names): for writers that read
/// the document tree themselves (plug-in exporters, legacy .ai). `None` when nothing has copies.
pub fn baked(doc: &Document) -> Option<Document> {
    if !has_stacks(doc) {
        return None;
    }
    let mut out = doc.clone();
    let layers = doc.layers.clone();
    let mut changed = false;
    out.layers = layers
        .iter()
        .map(|l| match bake_tree(doc, &mut out, l, 0) {
            Some(n) => {
                changed = true;
                Arc::new(n)
            }
            None => l.clone(),
        })
        .collect();
    changed.then_some(out)
}

/// `n` with the objects in it that have copies replaced by them (fresh ids from `out`), `None`
/// when nothing in it has copies. Copies are read from `doc` (copy-to-shape targets resolve there).
fn bake_tree(doc: &Document, out: &mut Document, n: &Node, depth: usize) -> Option<Node> {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return None;
    }
    if let Some(a) = art(doc, n) {
        let mut m = Arc::unwrap_or_clone(a);
        let mut seen = std::collections::HashSet::from([m.id]);
        for c in m.children_mut().into_iter().flatten() {
            let c = Arc::make_mut(c);
            // Members' own copies, inside each copy (their stacks were kept within the budget).
            if let Some(b) = bake_tree(doc, out, c, depth + 1) {
                *c = b;
            }
            vectorcraft_effects::fresh_ids(out, c, &mut seen);
        }
        return Some(m);
    }
    if has_stack(n) {
        // Active but no copies: the object as it is, without the stack.
        return Some(plain(n));
    }
    let children = n.children()?;
    let mut changed = false;
    let baked: Vec<Arc<Node>> = children
        .iter()
        .map(|c| match bake_tree(doc, out, c, depth + 1) {
            Some(b) => {
                changed = true;
                Arc::new(b)
            }
            None => c.clone(),
        })
        .collect();
    if !changed {
        return None;
    }
    let mut m = n.clone();
    if let Some(ch) = m.children_mut() {
        *ch = baked;
    }
    Some(m)
}

/// What may draw differently in `new` than in `old` because a copy-to-shape target changed while
/// its source didn't, or a parameter a bound object reads did: those objects' painted bounds before
/// and after. Looks only into the containers that hold such objects ([`subtree_flags`]): nothing
/// when the document has none.
pub(crate) fn dependents(old: &Document, new: &Document) -> Option<Rect> {
    let (os, ns) = (old.symbol_boxes(), new.symbol_boxes());
    // A bound object inside a symbol's art draws in every instance: when the parameters it reads
    // change, the layers are drawn again (rare: symbols with bound stacks).
    if symbols_read_changed(old, new) {
        let all = |d: &Document, s: &SymbolBoxes| d.layers.iter().filter_map(|l| crate::brush_fx::cull_bounds(s, l)).reduce(|a, b| a.union(b));
        return vectorcraft_geom::union_opt(all(new, &ns), all(old, &os));
    }
    if !doc_flags(new).targets {
        return None;
    }
    let mut area: Option<Rect> = None;
    let mut visit = |n: &Node| {
        if !has_targets(n) {
            return;
        }
        // The old document may be an undo step: no lookup cache is built for it.
        let moved = vectorcraft_modifiers::stack_targets(Some(n)).iter().any(|t| match (old.node_uncached(*t), new.node(*t)) {
            (Some(a), Some(b)) => !std::ptr::eq(a, b),
            (None, None) => false,
            _ => true,
        });
        // A bound object whose parameters changed (the object itself may not have).
        let moved = moved
            || (vectorcraft_modifiers::has_bindings(n)
                && old.node_uncached(n.id).map(|o| vectorcraft_modifiers::bound_key(old, o)) != Some(vectorcraft_modifiers::bound_key(new, n)));
        if moved {
            area = vectorcraft_geom::union_opt(area, crate::brush_fx::cull_bounds(&ns, n));
            area = vectorcraft_geom::union_opt(area, old.node_uncached(n.id).and_then(|o| crate::brush_fx::cull_bounds(&os, o)));
        }
    };
    fn walk(a: &Arc<Node>, depth: usize, visit: &mut dyn FnMut(&Node)) {
        if depth > vectorcraft_doc::MAX_DEPTH || !subtree_flags(a).targets {
            return;
        }
        visit(a);
        for c in a.children().into_iter().flatten() {
            walk(c, depth + 1, visit);
        }
    }
    for l in &new.layers {
        walk(l, 0, &mut visit);
    }
    area
}

/// Whether a bound object in a symbol's art reads a parameter whose value differs between `old`
/// and `new`. Nothing to look at when neither the parameters nor the variables changed.
fn symbols_read_changed(old: &Document, new: &Document) -> bool {
    if new.symbols.is_empty()
        || (vectorcraft_doc::Shared::ptr_eq(&old.unknown, &new.unknown) && vectorcraft_doc::Shared::ptr_eq(&old.variables, &new.variables))
    {
        return false;
    }
    let mut changed = false;
    for s in &new.symbols {
        s.art.walk(&mut |n: &Node| {
            if !changed && vectorcraft_modifiers::has_bindings(n) {
                changed = vectorcraft_modifiers::bound_key(old, n) != vectorcraft_modifiers::bound_key(new, n);
            }
        });
        if changed {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "picabia_tests.rs"]
mod tests;
