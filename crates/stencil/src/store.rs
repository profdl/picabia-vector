//! The computed stencil sheets, kept for the renderer, exports and commands.
//!
//! The engine computes (`vectorcraft_engine::picabia_stencil`: off the UI thread natively, after a
//! settle on wasm) and puts each result here with a snapshot of what it was computed from; the
//! renderer (same layer as this crate) and the engine read it back with [`lookup`] / [`drawn`]:
//! **fresh** when the document still matches the snapshot, else the object's latest result
//! (**stale**: drawn while the engine recomputes). An object with no result yet draws its design.
//!
//! Two keys:
//! - [`Inputs`], the cheap one every lookup checks: the object itself (the `Arc` the document
//!   holds: an unchanged object is the same pointer), its copies and repeat lattice, the copies of
//!   objects inside it whose copy-to-shape targets lie outside, and the symbols when it places
//!   any. Pointer comparisons and cached evaluations only, cheap enough for every frame.
//! - The geometry key ([`geometry_key`]): a hash of the stencil params (preview colours left out)
//!   and of the gathered outlines. When the cheap key misses, the engine gathers (off the UI
//!   thread) and, when the geometry key matches a kept result, only re-keys it ([`rekey`]): a
//!   preview colour change, an undo back to an earlier design or a reopened file costs a gather,
//!   not a compute.
//!
//! Every [`store`] and [`rekey`] bumps [`generation`]: the canvas folds it into its cache key so
//! a result that lands repaints.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_geom::kurbo::{BezPath, PathEl, Point, Rect, Shape as _};
use vectorcraft_modifiers::{Repeat, Stencil, StencilOutput, StencilPreview, StencilTieMode};

use crate::{ColourInfo, Gathered, Island, Problem, Stats};

/// The most results kept (one per object and design; the oldest dropped first).
const MAX_ENTRIES: usize = 128;
/// The most objects an [`Inputs`] snapshot walks when it is taken.
const MAX_INPUT_NODES: usize = 200_000;

/// Where a stencil object's sheet lies, worked out from its params and bounds (no geometry).
/// Document space throughout: VectorCraft objects have no transform of their own.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The stencil row's id.
    pub modifier: String,
    /// Its params, clamped.
    pub params: Stencil,
    /// The copies the rows above the stencil make (document space to document space); empty:
    /// the object as it is.
    pub copies: Vec<Affine>,
    /// The object's bounds through every copy: the design the stencil is cut from.
    pub layer_bounds: Rect,
    /// Millimetres per document unit (`widthMm / layer_bounds width`).
    pub mm_per_unit: f64,
    /// The sheet: `layer_bounds` grown by `marginMm`; with tile output (and a usable repeat) the
    /// tile's: [`Frame::cell`]'s bounds grown by the margin.
    pub sheet: Rect,
    /// The stack's repeat lattice ([`vectorcraft_modifiers::Output::repeat`]).
    pub repeat: Option<Repeat>,
    /// With tile output: the repeat cell the tile is cut from, its four corners.
    pub cell: Option<[Point; 4]>,
}

/// Whether `n`'s stack has an enabled stencil row (cheap: the parsed stack is cached).
pub fn is_stencil(n: &Node) -> bool {
    vectorcraft_modifiers::has_modifiers(n) && matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.stencil().is_some())
}

/// `n`'s stencil frame in `doc`; `None` when its stack has no enabled stencil row or the design
/// has no usable size.
pub fn frame(doc: &Document, n: &Node) -> Option<Frame> {
    let out = vectorcraft_modifiers::node_instances_in_doc(doc, n)?;
    let row = out.stencil.as_ref()?;
    let copies: Vec<Affine> = out.transforms().filter(|c| c.is_finite()).collect();
    let own = vectorcraft_modifiers::node_bounds_in_doc(doc, n);
    let layer_bounds = if copies.is_empty() { own } else { copies.iter().map(|c| c.transform_rect_bbox(own)).reduce(|a, b| a.union(b))? };
    let finite = [layer_bounds.x0, layer_bounds.y0, layer_bounds.x1, layer_bounds.y1].iter().all(|v| v.is_finite());
    let mm_per_unit = crate::mm_per_unit(row.params.width_mm, layer_bounds)?;
    if !finite {
        return None;
    }
    let margin = row.params.margin_mm / mm_per_unit;
    let mut sheet = layer_bounds.abs().inflate(margin, margin);
    if !sheet.is_finite() {
        return None;
    }
    let repeat = out.repeat;
    let mut cell = None;
    if row.params.output == StencilOutput::Tile
        && row.params.tie_mode != StencilTieMode::Pattern
        && let Some(c) = repeat.and_then(|r| crate::tile_cell(r, layer_bounds, margin))
    {
        let b = c.iter().fold(Rect::from_points(c[0], c[0]), |b, p| b.union_pt(*p)).inflate(margin, margin);
        if b.is_finite() {
            sheet = b;
            cell = Some(c);
        }
    }
    Some(Frame { modifier: row.modifier.clone(), params: row.params.clone(), copies, layer_bounds, mm_per_unit, sheet, repeat, cell })
}

/// What `modifier.stencilInfo` and the stencil panel show, and the canvas markers' geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// The stencil row's id.
    pub modifier: String,
    /// Every colour of the design, largest visible area first.
    pub colours: Vec<ColourInfo>,
    pub warnings: Vec<String>,
    pub stats: Stats,
    /// The whole design's sheet (its bounds plus the margin), mm.
    pub sheet_mm: (f64, f64),
    /// With tile output: the tile's sheet, mm (what is drawn and exported).
    pub tile_mm: Option<(f64, f64)>,
    /// Shapes gathered from the design.
    pub paints: usize,
    /// Every paper island of the cut sheet, outermost first.
    pub islands: Vec<Island>,
    /// The tie bars (already part of the paper), for the canvas markers.
    pub ties: BezPath,
    /// What still breaks a cutting limit, at most [`crate::MAX_PROBLEMS`].
    pub problems: Vec<Problem>,
    /// The registration holes' centres.
    pub hoshi: Vec<Point>,
    /// The lattice the islands and ties are repeat-aware along; `None` when there's no repeat or
    /// the stencil ignores it (see the warnings).
    pub repeat: Option<Repeat>,
    /// The repeat cell shown (the tile's with tile output), its four corners; `Some` exactly when
    /// [`Info::repeat`] is.
    pub cell: Option<[Point; 4]>,
}

/// A computed stencil: what draws in place of the object's design, in document space.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    /// The cut-outs (even-odd; non-zero reads the same).
    pub cut: BezPath,
    /// The sheet minus the cut (even-odd).
    pub paper: BezPath,
    /// The sheet rectangle (the tile's with tile output).
    pub sheet: Rect,
    pub info: Info,
}

/// What a stencil was computed from, the cheap way (see the module docs).
#[derive(Clone, Debug)]
pub struct Inputs {
    node: Arc<Node>,
    copies: Vec<Affine>,
    repeat: Option<Repeat>,
    /// Objects inside with copy-to-shape targets (which may lie outside): their copies.
    inner: Vec<(Arc<Node>, Vec<Affine>)>,
    /// The symbols the design places (also inside other symbols' art): name and art (`None`:
    /// no such symbol).
    symbols: Vec<(String, Option<Arc<Node>>)>,
}

fn copies_of(doc: &Document, n: &Node) -> (Vec<Affine>, Option<Repeat>) {
    match vectorcraft_modifiers::node_instances_in_doc(doc, n) {
        Some(o) => (o.transforms().collect(), o.repeat),
        None => (Vec::new(), None),
    }
}

fn symbol_art<'a>(doc: &'a Document, name: &str) -> Option<&'a Arc<Node>> {
    doc.symbols.iter().find(|s| s.name == name).map(|s| &s.art)
}

fn has_targets(n: &Node) -> bool {
    vectorcraft_modifiers::has_modifiers(n) && matches!(vectorcraft_modifiers::node_stack(n), Some(Ok(s)) if s.targets().next().is_some())
}

fn same_affines(a: &[Affine], b: &[Affine]) -> bool {
    // Bit-exact would recompute on last-bit noise; NaN never matches (but is never stored: copies
    // are finite).
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * (1.0 + x.abs().max(y.abs()));
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.as_coeffs().iter().zip(q.as_coeffs()).all(|(x, y)| close(*x, y)))
}

fn same_repeat(a: Option<Repeat>, b: Option<Repeat>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let t = |r: Repeat| Affine::new([r.a.x, r.a.y, r.b.x, r.b.y, r.origin.x, r.origin.y]);
            same_affines(&[t(a)], &[t(b)])
        }
        _ => false,
    }
}

impl Inputs {
    /// What object `node` (the `Arc` `doc` holds) would be computed from now.
    pub fn take(doc: &Document, node: &Arc<Node>) -> Inputs {
        let (copies, repeat) = copies_of(doc, node);
        let mut inner = Vec::new();
        let mut symbols: Vec<(String, Option<Arc<Node>>)> = Vec::new();
        let mut stack: Vec<(&Arc<Node>, bool)> = vec![(node, true)];
        let mut seen = 0usize;
        while let Some((c, root)) = stack.pop() {
            seen += 1;
            if seen > MAX_INPUT_NODES {
                break;
            }
            if let NodeKind::SymbolInstance { symbol, .. } = &c.kind
                && !symbols.iter().any(|(n, _)| n == symbol)
            {
                let art = symbol_art(doc, symbol);
                if let Some(a) = art {
                    stack.push((a, false));
                }
                symbols.push((symbol.clone(), art.cloned()));
            }
            if !root && has_targets(c) {
                inner.push((c.clone(), copies_of(doc, c).0));
            }
            if let Some(kids) = c.children() {
                stack.extend(kids.iter().rev().map(|k| (k, false)));
            }
        }
        Inputs { node: node.clone(), copies, repeat, inner, symbols }
    }

    /// Whether object `n` of `doc` still matches this snapshot.
    pub fn matches(&self, doc: &Document, n: &Node) -> bool {
        if !std::ptr::eq(Arc::as_ptr(&self.node), n) {
            return false;
        }
        let same_art = |a: Option<&Arc<Node>>, b: Option<&Arc<Node>>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !self.symbols.iter().all(|(name, art)| same_art(art.as_ref(), symbol_art(doc, name))) {
            return false;
        }
        let (copies, repeat) = copies_of(doc, n);
        if !same_affines(&self.copies, &copies) || !same_repeat(self.repeat, repeat) {
            return false;
        }
        self.inner.iter().all(|(c, stored)| same_affines(stored, &copies_of(doc, c).0))
    }

    /// The object this snapshot was taken of.
    pub fn node(&self) -> &Arc<Node> {
        &self.node
    }
}

/// The geometry key of a stencil: its params (the preview colours left out: they don't change
/// the sheet), its frame and the gathered outlines.
pub fn geometry_key(frame: &Frame, gathered: &Gathered) -> u64 {
    let mut h = DefaultHasher::new();
    let d = Stencil::default();
    let params = Stencil {
        preview: d.preview,
        paper_colour: d.paper_colour.clone(),
        paste_colour: d.paste_colour.clone(),
        cloth_colour: d.cloth_colour.clone(),
        ..frame.params.clone()
    };
    format!("{params:?}").hash(&mut h);
    let f = |h: &mut DefaultHasher, v: f64| v.to_bits().hash(h);
    for v in [frame.layer_bounds.x0, frame.layer_bounds.y0, frame.layer_bounds.x1, frame.layer_bounds.y1] {
        f(&mut h, v);
    }
    match frame.repeat {
        None => 0u8.hash(&mut h),
        Some(r) => {
            1u8.hash(&mut h);
            for v in [r.origin.x, r.origin.y, r.a.x, r.a.y, r.b.x, r.b.y] {
                f(&mut h, v);
            }
        }
    }
    gathered.paints.len().hash(&mut h);
    for p in &gathered.paints {
        p.order.hash(&mut h);
        for c in p.colour {
            c.to_bits().hash(&mut h);
        }
        p.path.elements().len().hash(&mut h);
        for el in p.path.elements() {
            let pts: &[Point] = match el {
                PathEl::MoveTo(a) => {
                    0u8.hash(&mut h);
                    std::slice::from_ref(a)
                }
                PathEl::LineTo(a) => {
                    1u8.hash(&mut h);
                    std::slice::from_ref(a)
                }
                PathEl::QuadTo(a, b) => {
                    2u8.hash(&mut h);
                    f(&mut h, a.x);
                    f(&mut h, a.y);
                    std::slice::from_ref(b)
                }
                PathEl::CurveTo(a, b, c) => {
                    3u8.hash(&mut h);
                    for q in [a, b] {
                        f(&mut h, q.x);
                        f(&mut h, q.y);
                    }
                    std::slice::from_ref(c)
                }
                PathEl::ClosePath => {
                    4u8.hash(&mut h);
                    &[]
                }
            };
            for q in pts {
                f(&mut h, q.x);
                f(&mut h, q.y);
            }
        }
    }
    gathered.warnings.hash(&mut h);
    h.finish()
}

struct Entry {
    id: NodeId,
    inputs: Inputs,
    key: u64,
    sheet: Arc<Sheet>,
    stamp: u64,
}

#[derive(Default)]
struct Store {
    entries: Vec<Entry>,
    stamp: u64,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn lock() -> MutexGuard<'static, Option<Store>> {
    // A panic elsewhere while holding the lock leaves plain data: keep using it.
    STORE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Bumped by every [`store`], [`rekey`] and [`clear`]: a repaint key for the canvas.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn bump() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Keep `sheet`, object `id`'s stencil computed from `inputs` with geometry `key`. Replaces a
/// result kept for the same object and key; the oldest results go past the store's cap.
pub fn store(id: NodeId, inputs: Inputs, key: u64, sheet: Sheet) -> Arc<Sheet> {
    let sheet = Arc::new(sheet);
    {
        let mut g = lock();
        let s = g.get_or_insert_with(Store::default);
        s.stamp += 1;
        let stamp = s.stamp;
        s.entries.retain(|e| !(e.id == id && e.key == key));
        s.entries.push(Entry { id, inputs, key, sheet: sheet.clone(), stamp });
        while s.entries.len() > MAX_ENTRIES {
            let Some((oldest, _)) = s.entries.iter().enumerate().min_by_key(|(_, e)| e.stamp) else { break };
            s.entries.swap_remove(oldest);
        }
    }
    bump();
    sheet
}

/// A kept result of object `id` with geometry `key`, now answering to `inputs` (and the newest);
/// `None` when there's none (compute it).
pub fn rekey(id: NodeId, key: u64, inputs: Inputs) -> Option<Arc<Sheet>> {
    let sheet = {
        let mut g = lock();
        let s = g.as_mut()?;
        s.stamp += 1;
        let stamp = s.stamp;
        let e = s.entries.iter_mut().find(|e| e.id == id && e.key == key)?;
        e.inputs = inputs;
        e.stamp = stamp;
        e.sheet.clone()
    };
    bump();
    Some(sheet)
}

/// Object `n`'s stencil in `doc`: the result computed from what `doc` holds now (`true`: fresh)
/// or its latest one (`false`: stale, being recomputed). `None` when none was ever computed.
pub fn lookup(doc: &Document, n: &Node) -> Option<(Arc<Sheet>, bool)> {
    let g = lock();
    let s = g.as_ref()?;
    let mut latest: Option<&Entry> = None;
    for e in s.entries.iter().filter(|e| e.id == n.id) {
        if e.inputs.matches(doc, n) {
            return Some((e.sheet.clone(), true));
        }
        if latest.is_none_or(|l| e.stamp > l.stamp) {
            latest = Some(e);
        }
    }
    latest.map(|e| (e.sheet.clone(), false))
}

/// Whether object `n` has a result computed from what `doc` holds now.
pub fn is_fresh(doc: &Document, n: &Node) -> bool {
    lookup(doc, n).is_some_and(|(_, fresh)| fresh)
}

/// Forget every result (tests).
pub fn clear() {
    *lock() = None;
    bump();
}

/// What draws in place of a stencil object's design: its frame, the result (fresh or stale).
pub struct Drawn {
    pub frame: Frame,
    pub sheet: Arc<Sheet>,
    pub fresh: bool,
}

/// [`Drawn`] for `n`, when it is a stencil object with a result; `None`: draw it as usual (not a
/// stencil, or nothing computed yet).
pub fn drawn(doc: &Document, n: &Node) -> Option<Drawn> {
    let frame = frame(doc, n)?;
    let (sheet, fresh) = lookup(doc, n)?;
    Some(Drawn { frame, sheet, fresh })
}

/// One filled area of a stencil preview: a path in document space, its fill rule (`true`:
/// even-odd) and colour (sRGB bytes).
pub struct Fill<'a> {
    pub path: std::borrow::Cow<'a, BezPath>,
    pub even_odd: bool,
    pub rgb: [u8; 3],
}

/// What the preview paints, bottom to top: `paper` = the paper with the cut-outs open (the canvas
/// shows through); `cloth` = the cloth with the paste where the cut-outs let it through.
pub fn fills(d: &Drawn) -> Vec<Fill<'_>> {
    let p = &d.frame.params;
    let col = |hex: &str, fallback: &str| {
        vectorcraft_modifiers::parse_hex_colour(hex)
            .or_else(|| vectorcraft_modifiers::parse_hex_colour(fallback))
            .map_or([0, 0, 0], |c| [c[0], c[1], c[2]])
    };
    match p.preview {
        StencilPreview::Paper => vec![Fill {
            path: std::borrow::Cow::Borrowed(&d.sheet.paper),
            even_odd: true,
            rgb: col(&p.paper_colour, vectorcraft_modifiers::STENCIL_PAPER),
        }],
        StencilPreview::Cloth => vec![
            Fill {
                path: std::borrow::Cow::Owned(d.sheet.sheet.to_path(0.1)),
                even_odd: false,
                rgb: col(&p.cloth_colour, vectorcraft_modifiers::STENCIL_CLOTH),
            },
            Fill { path: std::borrow::Cow::Borrowed(&d.sheet.cut), even_odd: true, rgb: col(&p.paste_colour, vectorcraft_modifiers::STENCIL_PASTE) },
        ],
    }
}
