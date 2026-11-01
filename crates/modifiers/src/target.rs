//! Copy-to-shape targets: another layer's outline, read in the source layer's parent space.
//!
//! [`crate::evaluate`] alone has no document, so a `copyToShape` modifier evaluated through it
//! places no copies (with a warning). [`crate::evaluate_with`] takes a [`TargetResolver`];
//! [`DocTargets`] is the one built from a [`Document`] that the renderer, exports, hit testing and
//! commands use. A target is refused (no copies, a warning) when it is missing, a top-level layer,
//! the source itself, an ancestor or descendant of the source, or when its own stack points back at
//! the source, directly or through other targets ([`check_target`]). The engine refuses
//! such targets when they are set; evaluation ignores them in files that have them anyway.
//!
//! The target's outline is its own geometry in document space (a path's path; a group, compound
//! path or sublayer is the outlines of its visible members), not its modifier copies.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{PathEl, Shape};
use vectorcraft_geom::{Affine, BezPath};

use crate::cache;
use crate::node::node_source;
use crate::screen::ToneImage;

/// The most path elements a target outline may have (bigger outlines are refused).
pub const MAX_TARGET_ELEMENTS: usize = 100_000;
/// How deep a group target's layers are read.
const MAX_GROUP_DEPTH: usize = 16;
/// The most stacks a cycle check visits.
const MAX_CYCLE_VISITS: usize = 1024;
/// The most layers [`valid_targets`] lists.
pub const MAX_LISTED_TARGETS: usize = 10_000;

/// A resolved target: its outline in the source's parent space and a hash of it (part of the
/// evaluation cache key, so moving or editing the target re-evaluates its sources).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetShape {
    pub outline: BezPath,
    pub hash: u64,
}

impl TargetShape {
    /// A target shape for `outline`, hashed.
    pub fn new(outline: BezPath) -> TargetShape {
        let hash = hash_path(&outline);
        TargetShape { outline, hash }
    }
}

/// Finds a `copyToShape` modifier's target, and a `screen` row's image. `Err` is the warning to
/// show (no copies are placed; the screen is skipped).
pub trait TargetResolver {
    fn resolve(&self, target: &str) -> Result<Arc<TargetShape>, String>;

    /// The darkness of image object `target`, for a `screen` row whose tone comes from an image.
    fn tone(&self, _target: &str) -> Result<Arc<ToneTarget>, String> {
        Err("a screen needs the document to find its image; it is skipped".into())
    }
}

/// No document: every target is unresolved.
pub struct NoTargets;

impl TargetResolver for NoTargets {
    fn resolve(&self, _target: &str) -> Result<Arc<TargetShape>, String> {
        Err("copy to shape needs the document to find its target; no copies".into())
    }
}

/// A `screen` row's image, resolved: its darkness ([`ToneImage`], at most
/// [`crate::screen::MAX_TONE_PIXELS`]: a bigger image is averaged down to fit) and the map from
/// document space into its pixel space, so a rotated or skewed image is read where it shows.
#[derive(Clone)]
pub struct ToneTarget {
    pub image: Arc<ToneImage>,
    /// Document space → the image object's pixel space (`0..width × 0..height`).
    pub to_pixels: Affine,
    /// Identifies the pixels and the placement: part of the evaluation cache key.
    pub key: u64,
}

impl ToneTarget {
    /// The image's darkness at document point `p` (0 outside it, where transparent, or when `p`
    /// isn't finite).
    pub fn darkness_at(&self, p: vectorcraft_geom::Point) -> f64 {
        self.image.darkness_at(self.to_pixels * p)
    }
}

/// Compared by key: the pixels are never compared one by one.
impl PartialEq for ToneTarget {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// The size and key, not the pixels.
impl std::fmt::Debug for ToneTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (w, h) = self.image.size();
        f.debug_struct("ToneTarget").field("size", &(w, h)).field("to_pixels", &self.to_pixels).field("key", &self.key).finish()
    }
}

/// Targets read from `doc` for the layer `source`.
pub struct DocTargets<'a> {
    pub doc: &'a Document,
    pub source: NodeId,
}

impl<'a> DocTargets<'a> {
    pub fn new(doc: &'a Document, source: NodeId) -> Self {
        DocTargets { doc, source }
    }
}

impl TargetResolver for DocTargets<'_> {
    fn resolve(&self, target: &str) -> Result<Arc<TargetShape>, String> {
        let t = parse_target(target)?;
        check_target(self.doc, self.source, t)?;
        target_outline(self.doc, self.source, t).map(|o| Arc::new(TargetShape::new(o)))
    }

    fn tone(&self, target: &str) -> Result<Arc<ToneTarget>, String> {
        let t = parse_target(target)?;
        check_target(self.doc, self.source, t)?;
        image_tone(self.doc, t)
    }
}

/// How many decoded images [`image_tone`] keeps.
const KEPT_TONES: usize = 4;

/// `(pixel bytes held, their address and length, decoded size, darkness)`: holding the bytes keeps
/// their address from being reused by other pixels while the entry lives.
type KeptTone = (Arc<Vec<u8>>, (usize, usize, u32, u32), Arc<ToneImage>);

static TONES: Mutex<Vec<KeptTone>> = Mutex::new(Vec::new());

/// Image object `id`'s darkness. Decoded once per distinct pixels (the last [`KEPT_TONES`] kept),
/// so evaluating a stack every frame only looks the image up.
fn image_tone(doc: &Document, id: NodeId) -> Result<Arc<ToneTarget>, String> {
    let n = doc.node(id).ok_or("the screen's image is gone; it is skipped")?;
    let NodeKind::Image(im) = &n.kind else {
        return Err(format!("\"{}\" isn't an image; the screen is skipped", label(n)));
    };
    let det = im.xf.determinant();
    if !(det.is_finite() && det.abs() > 1e-12 && im.xf.as_coeffs().iter().all(|c| c.is_finite())) {
        return Err(format!("\"{}\" is placed flat or off the page; the screen is skipped", label(n)));
    }
    let blob = doc.image_blob(im).ok_or_else(|| format!("\"{}\" has no pixels; the screen is skipped", label(n)))?;
    let id_of = (Arc::as_ptr(&blob.bytes) as usize, blob.bytes.len(), im.width, im.height);
    let kept = {
        let tones = TONES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        tones.iter().find(|(_, k, _)| *k == id_of).map(|(_, _, t)| t.clone())
    };
    let image = match kept {
        Some(t) => t,
        None => {
            let place = vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im.width.max(1)), f64::from(im.height.max(1)));
            let (w, h, cover) = blob.coverage().ok_or_else(|| format!("\"{}\" couldn't be read; the screen is skipped", label(n)))?;
            let (w, h, luma) = fit_pixels(w as usize, h as usize, &cover);
            let t = Arc::new(
                ToneImage::from_luma8(w, h, &luma, place).ok_or_else(|| format!("\"{}\" has no pixels to read; the screen is skipped", label(n)))?,
            );
            let mut tones = TONES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tones.retain(|(_, k, _)| *k != id_of);
            tones.insert(0, (blob.bytes.clone(), id_of, t.clone()));
            tones.truncate(KEPT_TONES);
            t
        }
    };
    let mut h = DefaultHasher::new();
    id_of.hash(&mut h);
    for c in im.xf.as_coeffs() {
        c.to_bits().hash(&mut h);
    }
    Ok(Arc::new(ToneTarget { image, to_pixels: im.xf.inverse(), key: h.finish() }))
}

/// Coverage bytes (`w × h`, darkness 0..255) as grey levels (255 white) of at most
/// [`crate::screen::MAX_TONE_PIXELS`]: averaged over `k × k` blocks when bigger.
fn fit_pixels(w: usize, h: usize, cover: &[u8]) -> (usize, usize, Vec<u8>) {
    let n = w.saturating_mul(h);
    if n <= crate::screen::MAX_TONE_PIXELS && cover.len() == n {
        return (w, h, cover.iter().map(|c| 255 - c).collect());
    }
    if w == 0 || h == 0 || cover.len() != n {
        return (0, 0, Vec::new());
    }
    let mut k = ((n as f64 / crate::screen::MAX_TONE_PIXELS as f64).sqrt().ceil() as usize).max(2);
    while w.div_ceil(k).saturating_mul(h.div_ceil(k)) > crate::screen::MAX_TONE_PIXELS {
        k += 1;
    }
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::with_capacity(sw * sh);
    for by in 0..sh {
        for bx in 0..sw {
            let (mut sum, mut count) = (0u64, 0u64);
            for y in by * k..((by + 1) * k).min(h) {
                let row = cover.get(y * w + bx * k..y * w + ((bx + 1) * k).min(w)).unwrap_or_default();
                sum += row.iter().map(|&c| u64::from(c)).sum::<u64>();
                count += row.len() as u64;
            }
            let mean = sum.checked_div(count).unwrap_or(0);
            out.push(255 - u8::try_from(mean.min(255)).unwrap_or(255));
        }
    }
    (sw, sh, out)
}

/// The object id a `target` text names (`"12"`, or `"#12"` as VectorCraft prints ids).
pub fn parse_target(target: &str) -> Result<NodeId, String> {
    let t = target.trim();
    t.strip_prefix('#')
        .unwrap_or(t)
        .parse::<u64>()
        .map(NodeId)
        .map_err(|_| format!("`{}` isn't an object id", crate::model::short(&serde_json::Value::String(target.into()))))
}

/// `n`'s display name for messages.
fn label(n: &Node) -> &str {
    n.name.as_deref().unwrap_or("the target")
}

/// Whether `a` is a strict ancestor of `b`.
fn is_ancestor(doc: &Document, a: NodeId, b: NodeId) -> bool {
    a != b && doc.ancestry(b).is_some_and(|chain| chain.contains(&a))
}

/// Whether layer `target` can be layer `source`'s copy-to-shape target; `Err` says why not.
pub fn check_target(doc: &Document, source: NodeId, target: NodeId) -> Result<(), String> {
    let Some(t) = doc.node(target) else { return Err("the target layer is gone (deleted, or not in this file); no copies".into()) };
    if doc.parent_of(target).is_none() {
        return Err("a top-level layer can't be a copy-to-shape target".into());
    }
    if target == source {
        return Err("an object can't copy onto itself".into());
    }
    if is_ancestor(doc, target, source) {
        return Err(format!("\"{}\" contains this object, so it can't be its target", label(t)));
    }
    if is_ancestor(doc, source, target) {
        return Err(format!("\"{}\" is inside this object, so it can't be its target", label(t)));
    }
    if reaches(doc, target, source) {
        return Err(format!("\"{}\" copies onto this object itself (directly or through other objects), so it can't be its target", label(t)));
    }
    Ok(())
}

/// Whether following copy-to-shape targets from `from`'s stack reaches `to`.
fn reaches(doc: &Document, from: NodeId, to: NodeId) -> bool {
    let mut seen = BTreeSet::new();
    let mut todo = vec![from];
    while let Some(id) = todo.pop() {
        if !seen.insert(id) || seen.len() > MAX_CYCLE_VISITS {
            continue;
        }
        for t in stack_targets(doc.node(id)) {
            if t == to {
                return true;
            }
            todo.push(t);
        }
    }
    false
}

/// The layer ids `n`'s stack copies onto (enabled modifiers; unreadable ids left out).
pub fn stack_targets(n: Option<&Node>) -> Vec<NodeId> {
    let Some(src) = n.and_then(node_source) else { return Vec::new() };
    let Ok(stack) = cache::parsed(src) else { return Vec::new() };
    stack.targets().filter_map(|(_, t)| parse_target(t).ok()).collect()
}

/// The objects `source` can copy onto: the objects in the document passing [`check_target`] that
/// have an outline, in layer order (at most [`MAX_LISTED_TARGETS`]).
pub fn valid_targets(doc: &Document, source: NodeId) -> Vec<NodeId> {
    let mut ids = Vec::new();
    doc.walk(|n| ids.push(n.id));
    ids.into_iter()
        .filter(|id| check_target(doc, source, *id).is_ok())
        .filter(|id| doc.node(*id).and_then(|n| local_outline(n, 0)).is_some_and(|p| !p.elements().is_empty()))
        .take(MAX_LISTED_TARGETS)
        .collect()
}

/// `target`'s outline in document space (where `source`'s geometry also lives).
fn target_outline(doc: &Document, _source: NodeId, target: NodeId) -> Result<BezPath, String> {
    let t = doc.node(target).ok_or("the target object is gone; no copies")?;
    let out = local_outline(t, 0).ok_or_else(|| format!("\"{}\" has no outline to copy onto", label(t)))?;
    if out.elements().len() > MAX_TARGET_ELEMENTS {
        return Err(format!("\"{}\" has more than {MAX_TARGET_ELEMENTS} path points", label(t)));
    }
    let finite = out.elements().iter().all(|e| el_points(e).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    if !finite || out.elements().is_empty() {
        return Err(format!("\"{}\" has no outline to copy onto", label(t)));
    }
    Ok(out)
}

/// `n`'s outline in document space: its path; for a clip group or clipped sublayer its clipping
/// path; for a group, compound path or sublayer its visible members' outlines; for any other
/// object (text, an image, a symbol instance, a placed document, a mesh…) its geometric box, as
/// picabia-craft read a text or frame target.
fn local_outline(n: &Node, depth: usize) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Path { path, .. } => return Some(path.to_bezpath()),
        NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
            return if depth >= MAX_GROUP_DEPTH { None } else { children.first().and_then(|c| local_outline(c, depth + 1)) };
        }
        NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Compound { .. } => {}
        _ => {
            let r = n.geometric_bounds()?;
            let finite = r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite();
            return (finite && (r.width() > 0.0 || r.height() > 0.0)).then(|| r.to_path(0.1));
        }
    }
    if depth >= MAX_GROUP_DEPTH {
        return None;
    }
    let mut out = BezPath::new();
    for c in n.children().map(Vec::as_slice).unwrap_or_default() {
        if !c.visible {
            continue;
        }
        let Some(p) = local_outline(c, depth + 1) else { continue };
        if out.elements().len() + p.elements().len() > MAX_TARGET_ELEMENTS {
            break;
        }
        out.extend(p.elements().iter().copied());
    }
    (!out.elements().is_empty()).then_some(out)
}

fn el_points(e: &PathEl) -> Vec<vectorcraft_geom::Point> {
    match *e {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => Vec::new(),
    }
}

fn hash_path(p: &BezPath) -> u64 {
    let mut h = DefaultHasher::new();
    for e in p.elements() {
        std::mem::discriminant(e).hash(&mut h);
        for q in el_points(e) {
            q.x.to_bits().hash(&mut h);
            q.y.to_bits().hash(&mut h);
        }
    }
    h.finish()
}
