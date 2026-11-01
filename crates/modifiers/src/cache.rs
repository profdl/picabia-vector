//! Process-wide caches: parsed stacks by source text, evaluated outputs by (source, bounds,
//! resolved copy-to-shape targets).
//!
//! One `Mutex` (works the same on wasm32, where there is one thread). Both maps are bounded; when
//! one is full it is cleared, which is simple and fine for a cache that refills in microseconds.
//! A poisoned lock is recovered: the caches hold no invariants a panic could break.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use vectorcraft_geom::Rect;

use crate::eval::{Output, evaluate_with};
use crate::model::Stack;
use crate::target::{NoTargets, TargetResolver, TargetShape, ToneTarget};
use crate::{ModError, parse};

/// The most stacks kept parsed, and the most source bytes they may hold.
const MAX_PARSED: usize = 1024;
const MAX_PARSED_BYTES: usize = 8 * 1024 * 1024;
/// The most outputs kept, and the most instances they may hold in all.
const MAX_OUTPUTS: usize = 1024;
const MAX_OUTPUT_INSTANCES: usize = 200_000;
/// The most image darkness bytes the kept outputs may hold (each output counted alone, so a shared
/// image counts once per output: an over-count that only clears the cache sooner).
const MAX_OUTPUT_TONE_BYTES: usize = if cfg!(target_arch = "wasm32") { 64 << 20 } else { 256 << 20 };

struct Parsed {
    src: Box<str>,
    stack: Result<Arc<Stack>, ModError>,
}

#[derive(Default)]
struct Caches {
    parsed: HashMap<u64, Parsed>,
    parsed_bytes: usize,
    outputs: HashMap<(u64, usize, [u64; 4], u64), Arc<Output>>,
    output_instances: usize,
    output_tone_bytes: usize,
}

static CACHES: Mutex<Option<Caches>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Caches>> {
    CACHES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn hash(src: &str) -> u64 {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    h.finish()
}

/// [`parse`] once per distinct source.
pub(crate) fn parsed(src: &str) -> Result<Arc<Stack>, ModError> {
    let key = hash(src);
    if let Some(c) = lock().as_ref()
        && let Some(p) = c.parsed.get(&key)
        && &*p.src == src
    {
        return p.stack.clone();
    }
    // Parsed outside the lock: another thread may parse the same source meanwhile (harmless).
    let stack = parse(src).map(Arc::new);
    let mut guard = lock();
    let c = guard.get_or_insert_with(Caches::default);
    if c.parsed.len() >= MAX_PARSED || c.parsed_bytes + src.len() > MAX_PARSED_BYTES {
        c.parsed.clear();
        c.parsed_bytes = 0;
    }
    if src.len() <= MAX_PARSED_BYTES {
        c.parsed_bytes += src.len();
        if let Some(old) = c.parsed.insert(key, Parsed { src: src.into(), stack: stack.clone() }) {
            c.parsed_bytes = c.parsed_bytes.saturating_sub(old.src.len());
        }
    }
    stack
}

/// [`crate::evaluate`] once per distinct (source, bounds). `Err` when the source doesn't parse.
pub(crate) fn evaluated(src: &str, bounds: Rect) -> Result<Arc<Output>, ModError> {
    evaluated_with(src, bounds, &NoTargets)
}

/// A target resolved once: its outline (or why not).
type ResolvedShape = (String, Result<Arc<TargetShape>, String>);
/// A screen's image resolved once: its darkness (or why not).
type ResolvedTone = (String, Result<Arc<ToneTarget>, String>);

/// Targets and screen images resolved once, up front: the cache key hashes them, and evaluation
/// reads them here.
struct Resolved(Vec<ResolvedShape>, Vec<ResolvedTone>);

impl TargetResolver for Resolved {
    fn resolve(&self, target: &str) -> Result<Arc<TargetShape>, String> {
        self.0.iter().find(|(t, _)| t == target).map_or_else(|| Err("the target wasn't resolved".into()), |(_, r)| r.clone())
    }

    fn tone(&self, target: &str) -> Result<Arc<ToneTarget>, String> {
        self.1.iter().find(|(t, _)| t == target).map_or_else(|| Err("the screen's image wasn't resolved".into()), |(_, r)| r.clone())
    }
}

/// [`crate::evaluate_with`] once per distinct (source, bounds, resolved targets): each target's
/// outline hash (or the reason it didn't resolve) is part of the key. `Err` when the source
/// doesn't parse.
pub(crate) fn evaluated_with(src: &str, bounds: Rect, targets: &dyn TargetResolver) -> Result<Arc<Output>, ModError> {
    let stack = parsed(src)?;
    let mut resolved: Vec<ResolvedShape> = Vec::new();
    for (_, t) in stack.shape_targets() {
        if !resolved.iter().any(|(r, _)| r == t) {
            resolved.push((t.to_string(), targets.resolve(t)));
        }
    }
    let mut tones: Vec<ResolvedTone> = Vec::new();
    for t in stack.screen_images() {
        if !tones.iter().any(|(r, _)| r == t) {
            tones.push((t.to_string(), targets.tone(t)));
        }
    }
    let mut h = DefaultHasher::new();
    for (t, r) in &resolved {
        t.hash(&mut h);
        match r {
            Ok(shape) => shape.hash.hash(&mut h),
            Err(e) => e.hash(&mut h),
        }
    }
    for (t, r) in &tones {
        t.hash(&mut h);
        match r {
            Ok(tone) => tone.key.hash(&mut h),
            Err(e) => e.hash(&mut h),
        }
    }
    let target_key = if resolved.is_empty() && tones.is_empty() { 0 } else { h.finish() };
    let key = (hash(src), src.len(), [bounds.x0.to_bits(), bounds.y0.to_bits(), bounds.x1.to_bits(), bounds.y1.to_bits()], target_key);
    if let Some(c) = lock().as_ref()
        && let Some(o) = c.outputs.get(&key)
    {
        return Ok(o.clone());
    }
    let out = Arc::new(evaluate_with(&stack, bounds, &Resolved(resolved, tones)));
    let mut guard = lock();
    let c = guard.get_or_insert_with(Caches::default);
    let n = out.instances.len();
    let bytes = tone_bytes(&out);
    if c.outputs.len() >= MAX_OUTPUTS
        || c.output_instances + n > MAX_OUTPUT_INSTANCES
        || c.output_tone_bytes.saturating_add(bytes) > MAX_OUTPUT_TONE_BYTES
    {
        c.outputs.clear();
        c.output_instances = 0;
        c.output_tone_bytes = 0;
    }
    if n <= MAX_OUTPUT_INSTANCES && bytes <= MAX_OUTPUT_TONE_BYTES {
        c.output_instances += n;
        c.output_tone_bytes += bytes;
        if let Some(old) = c.outputs.insert(key, out.clone()) {
            c.output_instances = c.output_instances.saturating_sub(old.instances.len());
            c.output_tone_bytes = c.output_tone_bytes.saturating_sub(tone_bytes(&old));
        }
    }
    Ok(out)
}

/// The bytes of the screen images `out` holds.
fn tone_bytes(out: &Output) -> usize {
    out.geometry
        .iter()
        .filter_map(|g| match &g.op {
            crate::shape::GeometryOp::Screen(r) => r.image.as_ref().and_then(|i| i.as_ref().ok()).map(|t| t.image.bytes()),
            _ => None,
        })
        .fold(0usize, usize::saturating_add)
}
