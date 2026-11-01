//! Evaluating a stack into instances.

use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bind::{BoundField, PerCopy};
use crate::kinds::{self, Ctx, Stage};
use crate::model::{ModifierKind, Stack, Stencil, StencilOutput, StencilRepeat};
use crate::params::Scope;
use crate::shape::{GeometryRow, RowAt};
use crate::target::{NoTargets, TargetResolver};
use crate::{MAX_INSTANCES_PER_NODE, MAX_LENGTH};

/// One drawn copy of a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// Applied in the parent's space before the node's own transform:
    /// `world = parent_world * transform * node.transform`. Identity = the original.
    pub transform: Affine,
    /// Flat index in [`Output::instances`] (`k * n_prev + j` after each array stage).
    pub index: u32,
    /// The copy number this instance has in each array stage, in stack order.
    pub stage_indices: Vec<u32>,
    /// A deterministic per-instance seed ([`splitmix64`] of the index); identical in wasm.
    pub seed: u64,
}

impl Instance {
    /// The original layer: identity transform, index 0.
    pub fn identity() -> Instance {
        Instance { transform: Affine::IDENTITY, index: 0, stage_indices: Vec::new(), seed: splitmix64(0) }
    }
}

/// A canvas guide a stage shows while its layer is selected, in the parent's space.
#[derive(Clone, Debug, PartialEq)]
pub enum Guide {
    Line { from: Point, to: Point },
    Circle { center: Point, radius: f64 },
    Point(Point),
}

/// Something worth telling the user about an evaluation (shown in the panel).
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// The modifier it concerns, when it's about one.
    pub modifier: Option<String>,
    pub message: String,
}

/// What a stack evaluates to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// The copies to draw. Empty = no active modifiers: draw the layer normally.
    pub instances: Vec<Instance>,
    pub guides: Vec<Guide>,
    pub warnings: Vec<Warning>,
    /// The stack's terminal stencil row, when it has an enabled one: the stack ends there and the
    /// engine and renderer turn what the copies above it draw into a stencil sheet. `None` = no
    /// stencil (draw the copies as they are).
    pub stencil: Option<StencilRow>,
    /// The translation lattice the stencil can repeat along: set only when the stack ends in an
    /// enabled stencil row with `repeat: auto` and the last array row above it is a `wallpaper` or
    /// `gridArray` whose copies still form its lattice (no array row after it, no `transformEach`
    /// or `randomize` row anywhere: those vary every copy). `None` otherwise, without a warning; a
    /// stencil with `output: tile` then evaluates as `sheet`, with a warning.
    pub repeat: Option<Repeat>,
    /// The enabled geometry rows (`resample`, `trim`, `tidy`, `deform`, `displace`), in stack order, each with where it
    /// runs: on the object before it is copied, or on every copy after its transform
    /// ([`crate::shape`]). Empty: the copies are the object's geometry, transformed.
    pub geometry: Vec<GeometryRow>,
    /// The rows that repeated the copies, in stack order, one per entry of
    /// [`Instance::stage_indices`]: whether each is an array (linear, circular, grid: the rows a
    /// deform's ramp steps over) and how many copies it made ([`Output::ramp_position`]).
    pub array_stages: Vec<ArrayStage>,
    /// What each bound field of the evaluated rows resolved to, in stack order ([`crate::bind`]).
    pub bound: Vec<BoundField>,
}

/// A row that repeated the copies ([`Output::array_stages`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayStage {
    /// A linear, circular or grid array (not a mirror, wallpaper or copy to shape).
    pub array: bool,
    /// How many copies of the set before it it made.
    pub copies: u32,
}

/// A translation lattice in the layer's parent space (like [`Instance::transform`]): the copies
/// repeat at `origin + i·a + j·b`. A zero vector means "no repeat along it" (a grid of one column
/// has `a` = 0, of one row `b` = 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Repeat {
    /// A lattice point: the wallpaper's symmetry origin, or the grid's top-left corner (the layer
    /// bounds' top-left).
    pub origin: Point,
    pub a: Vec2,
    pub b: Vec2,
}

/// The terminal stencil row of an evaluated stack ([`Output::stencil`]).
#[derive(Clone, Debug, PartialEq)]
pub struct StencilRow {
    /// The stencil modifier's id (warnings and commands name it).
    pub modifier: String,
    /// Its params, clamped ([`Stencil::clamped`]).
    pub params: Stencil,
}

impl Output {
    /// The instance transforms, in draw order.
    pub fn transforms(&self) -> impl Iterator<Item = Affine> + '_ {
        self.instances.iter().map(|i| i.transform)
    }

    /// Whether geometry rows change the paths: the copies must then be built ([`crate::Shaper`]),
    /// not just placed (and a single unmoved copy still differs from the object).
    pub fn has_geometry(&self) -> bool {
        !self.geometry.is_empty()
    }

    /// Where `inst` stands among the array copies (tlprocedural's progressive index): its copy
    /// number in the array rows only, stacked arrays read as one combined array (the first array's
    /// copies counting fastest, as [`Instance::index`] does), and how many array copies there are
    /// in all. Copies a mirror, wallpaper or copy to shape made share the position of the copy they
    /// came from. `None` without an array row, or with one array copy.
    pub fn ramp_position(&self, inst: &Instance) -> Option<(usize, usize)> {
        let (mut pos, mut total) = (0usize, 1usize);
        for (stage, k) in self.array_stages.iter().zip(&inst.stage_indices) {
            if stage.array {
                pos = pos.saturating_add((*k as usize).saturating_mul(total));
                total = total.saturating_mul(stage.copies as usize);
            }
        }
        (total > 1).then_some((pos.min(total - 1), total))
    }

    fn set_stencil(&mut self, id: &str, p: &Stencil) {
        let (params, warnings) = p.clamped();
        for w in warnings {
            self.warn(Some(id), w);
        }
        self.stencil = Some(StencilRow { modifier: id.to_string(), params });
    }

    /// The stencil row `m` the caps stopped short of, with its bound fields resolved.
    fn late_stencil(&mut self, m: &crate::Modifier, scope: &Scope, n_in: usize) {
        let r = crate::bind::resolve(m, scope, n_in);
        self.note_bound(r.fields);
        if let ModifierKind::Stencil(p) = &*r.kind {
            self.set_stencil(&m.id, p);
        }
    }

    /// Report the per-copy expressions that gave no number for some of the `n` copies (those used
    /// the field's own value), on their fields and as warnings.
    fn copy_failures(&mut self, copy_exprs: &[(String, PerCopy)], n: usize) {
        for (id, exprs) in copy_exprs {
            for e in &exprs.0 {
                let failed = e.failed();
                if failed == 0 {
                    continue;
                }
                let message = format!("`{}` gives no number for {failed} of {n} copies (a division by zero?); they use its own value", e.field);
                if let Some(b) = self.bound.iter_mut().find(|b| b.modifier == *id && b.field == e.field) {
                    b.error = Some(message.clone());
                }
                self.warn(Some(id), message);
            }
        }
    }

    /// Keep what a row's bound fields resolved to; a failed one is also a warning.
    fn note_bound(&mut self, fields: Vec<BoundField>) {
        for f in fields {
            if let Some(e) = &f.error {
                let message = format!("{}: {e}", f.field);
                self.warn(Some(&f.modifier), message);
            }
            self.bound.push(f);
        }
    }

    fn warn(&mut self, modifier: Option<&str>, message: impl Into<String>) {
        self.warnings.push(Warning { modifier: modifier.map(str::to_string), message: message.into() });
    }
}

/// SplitMix64: the crate's deterministic mixer for seeds (same result on every platform).
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn finite_rect(r: Rect) -> Option<Rect> {
    let ok = [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_LENGTH * 1e3);
    ok.then_some(r)
}

/// Evaluate `stack` for a layer whose bounding box in its parent's space is `bounds` (the pivot is
/// its centre).
///
/// Disabled modifiers are skipped; unknown kinds are skipped with a warning. Params are clamped
/// (see the crate's caps); a stage that would take the instance count past
/// [`MAX_INSTANCES_PER_NODE`] is not run, nor are the stages after it (with a warning). Never
/// panics; every returned transform is finite.
///
/// Without a document, `copyToShape` modifiers place no copies (with a warning): see
/// [`evaluate_with`].
pub fn evaluate(stack: &Stack, bounds: Rect) -> Output {
    evaluate_with(stack, bounds, &NoTargets)
}

/// [`evaluate`] with `targets` finding the layers `copyToShape` modifiers copy onto (in the
/// layer's parent space; see [`crate::DocTargets`]).
pub fn evaluate_with(stack: &Stack, bounds: Rect, targets: &dyn TargetResolver) -> Output {
    evaluate_in(stack, bounds, targets, &Scope::default())
}

/// [`evaluate_with`] with bound fields reading the parameters in `scope` ([`crate::bind`];
/// [`Scope::of_doc`] gives a document's). Without it every parameter is unknown: bound fields use
/// their own values, with a warning.
pub fn evaluate_in(stack: &Stack, bounds: Rect, targets: &dyn TargetResolver, scope: &Scope) -> Output {
    let mut out = Output::default();
    if !stack.is_active() {
        for m in stack.modifiers.iter().filter(|m| m.enabled) {
            if let ModifierKind::Other { kind, error } = &m.kind {
                out.warn(Some(&m.id), other_message(kind, error.as_deref()));
            }
        }
        return out;
    }
    let bounds = finite_rect(bounds).unwrap_or_else(|| {
        out.warn(None, "the layer's bounds aren't finite; copies are placed around the origin");
        Rect::ZERO
    });
    let pivot = bounds.center();
    let mut set = vec![Instance::identity()];
    let mut each: Vec<kinds::EachFn> = Vec::new();
    let mut rows = stack.modifiers.iter().filter(|m| m.enabled);
    // The last lattice row's repeat while the copies still form its lattice ([`Output::repeat`]).
    let mut lattice: Option<Repeat> = None;
    // A per-copy row anywhere varies the final copies one by one, so they never form a lattice.
    let mut per_copy = false;
    // A copy row has run: geometry rows from here on run on every copy.
    let mut copied = false;
    // The per-copy expressions, to report the copies they gave no number for once they have run.
    let mut copy_exprs: Vec<(String, PerCopy)> = Vec::new();
    for m in rows.by_ref() {
        // Bound fields take their values now: `n` is the copies the rows above made.
        let r = crate::bind::resolve(m, scope, set.len());
        out.note_bound(r.fields);
        let kind: &ModifierKind = &r.kind;
        // Terminal: no copies of its own, and the rows below it are ignored (one warning, below).
        if let ModifierKind::Stencil(p) = kind {
            out.set_stencil(&m.id, p);
            break;
        }
        if let ModifierKind::Other { kind, error } = kind {
            out.warn(Some(&m.id), other_message(kind, error.as_deref()));
            continue;
        }
        if let Some((op, warnings)) = crate::shape::clamped(kind) {
            for w in warnings {
                out.warn(Some(&m.id), w);
            }
            let at = if copied { RowAt::Copies } else { RowAt::Source };
            out.geometry.push(GeometryRow { modifier: m.id.clone(), op, at });
            continue;
        }
        copied = true;
        let ctx = Ctx { pivot, bounds, n_prev: set.len() };
        if !r.per_copy.0.is_empty() {
            copy_exprs.push((m.id.clone(), r.per_copy.clone()));
        }
        let st = kinds::stage(kind, &ctx, targets, &r.per_copy);
        out.guides.extend(st.guides);
        for w in st.warnings {
            out.warn(Some(&m.id), w);
        }
        match st.stage {
            Stage::Unchanged => {}
            Stage::Array(copies) => {
                let n = set.len().saturating_mul(copies.len());
                if n > MAX_INSTANCES_PER_NODE {
                    out.warn(
                        Some(&m.id),
                        format!("{n} copies is more than {MAX_INSTANCES_PER_NODE}; this modifier and the ones after it are skipped"),
                    );
                    break;
                }
                // An array of identities alone (one copy) doesn't move the lattice.
                if !copies.iter().all(|t| *t == Affine::IDENTITY) {
                    lattice = kinds::repeat(kind, &ctx);
                }
                let array = matches!(m.kind, ModifierKind::LinearArray(_) | ModifierKind::CircularArray(_) | ModifierKind::GridArray(_));
                out.array_stages.push(ArrayStage { array, copies: u32::try_from(copies.len()).unwrap_or(u32::MAX) });
                set = repeat_block(&set, &copies);
            }
            Stage::TooMany(copies) => {
                let n = set.len().saturating_mul(copies);
                out.warn(Some(&m.id), format!("{n} copies is more than {MAX_INSTANCES_PER_NODE}; this modifier and the ones after it are skipped"));
                break;
            }
            // Applied after every array stage has run (below), so each final copy gets its own
            // values wherever the row sits in the stack.
            Stage::Each(local) => {
                per_copy = true;
                each.push(local);
            }
        }
        let before = set.len();
        set.retain(|i| i.transform.is_finite());
        if set.len() != before {
            out.warn(Some(&m.id), format!("{} copies with invalid transforms were dropped", before - set.len()));
        }
        reindex(&mut set);
    }
    if out.stencil.is_none() {
        // A cap stopped the rows early: a stencil further down still ends the stack.
        if let Some(m) = rows.by_ref().find(|m| matches!(m.kind, ModifierKind::Stencil(_))) {
            out.late_stencil(m, scope, set.len());
        }
    }
    if let Some(st) = &mut out.stencil {
        let repeat = if st.params.repeat == StencilRepeat::Auto && !per_copy { lattice } else { None };
        let mut tile_without_repeat = None;
        if st.params.output == StencilOutput::Tile && repeat.is_none() {
            st.params.output = StencilOutput::Sheet;
            tile_without_repeat = Some(st.modifier.clone());
        }
        out.repeat = repeat;
        if let Some(id) = tile_without_repeat {
            out.warn(Some(&id), "tile output needs a repeat (a wallpaper or grid row above, with repeat on); cutting the whole sheet");
        }
    }
    if let Some(st) = &out.stencil {
        let below = rows.count();
        if below > 0 {
            let rows = if below == 1 { "1 row".to_string() } else { format!("{below} rows") };
            let id = st.modifier.clone();
            out.warn(Some(&id), format!("a stencil ends the stack: the {rows} below it are ignored"));
        }
    }
    // Per-copy stages see the final copies: `i` and `n` count all of them. Each acts in the copy's
    // own frame (`S_i · pivot · L · pivot⁻¹`), so where it sat among the arrays doesn't change
    // how it composes, only that every copy is treated separately.
    let n = set.len();
    for local in &each {
        for (i, inst) in set.iter_mut().enumerate() {
            let l = local(inst, i, n);
            inst.transform *= around(pivot, l);
        }
    }
    out.copy_failures(&copy_exprs, n);
    let before = set.len();
    set.retain(|i| i.transform.is_finite());
    if set.len() != before {
        out.warn(None, format!("{} copies with invalid transforms were dropped", before - set.len()));
    }
    reindex(&mut set);
    out.instances = set;
    out
}

fn other_message(kind: &str, error: Option<&str>) -> String {
    match error {
        Some(e) => format!("`{kind}` params don't read ({e}); ignored"),
        None if kind.is_empty() => "a modifier without a kind is ignored".into(),
        None => format!("unknown modifier kind `{kind}` is ignored"),
    }
}

/// `pivot · l · pivot⁻¹`.
pub(crate) fn around(pivot: Point, l: Affine) -> Affine {
    Affine::translate(pivot.to_vec2()) * l * Affine::translate(-pivot.to_vec2())
}

/// Every copy `T_k` of the whole previous block: `T_k * S_j` at index `k * n_prev + j`.
fn repeat_block(set: &[Instance], copies: &[Affine]) -> Vec<Instance> {
    let mut next = Vec::with_capacity(set.len() * copies.len());
    for (k, t) in copies.iter().enumerate() {
        let k = u32::try_from(k).unwrap_or(u32::MAX);
        for s in set {
            let mut stage_indices = Vec::with_capacity(s.stage_indices.len() + 1);
            stage_indices.extend_from_slice(&s.stage_indices);
            stage_indices.push(k);
            next.push(Instance { transform: *t * s.transform, index: 0, stage_indices, seed: 0 });
        }
    }
    next
}

fn reindex(set: &mut [Instance]) {
    for (i, inst) in set.iter_mut().enumerate() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        inst.index = i;
        inst.seed = splitmix64(u64::from(i));
    }
}
