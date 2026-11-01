//! PICABIA: the selection's modifier rows in the Appearance panel.
//!
//! The rows sit in the panel's one list, as a block between the fills and strokes and the object's
//! own effects (`panels::appearance::stack` calls [`rows`]): the stack makes the copies first,
//! then the object's fills, strokes and effects draw on each copy (`docs/picabia/modifiers.md`).
//! They are built from the panel's own row parts (eye, disclosure, label, the `fx`-style editor)
//! and VectorCraft's widgets, with colours from `theme::Tokens`; every edit goes through a
//! `modifier.*` command for every selected object, so it is one undo step and agents can do the
//! same.
//!
//! The rows are the whole selection's: when every selected object's stack has the same kinds in
//! the same order, one row per position (showing the first object's values, editing all of them);
//! when they differ, one "Mixed" row. A row's open state is kept by the first object and the
//! row's modifier id, so moving or removing rows keeps each open where it is.
//!
//! Each row: the eye turns it on and off (`modifier.toggle`), the disclosure opens its fields
//! (`modifier.set`, values clamped to each field's range; a stencil row opens the stencil
//! settings), the label names the kind and how many copies it makes, and the `⋯` menu moves,
//! duplicates, copies, removes it or applies the stack down to it. Under the rows: why the object
//! draws once if it does, the warnings the evaluation gives, the copy count and Apply. "Add New
//! Effect" (`fx`) lists the kinds under Modifiers ([`add_menu`]).

use std::sync::Arc;

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_modifiers::{Modifier, ModifierKind};

use crate::panels::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

const ROW: f32 = 30.0;
/// The eye column's width, as the Appearance panel's.
const EYE_W: f32 = 26.0;
/// The most objects the target picker lists.
const MAX_TARGETS: usize = 300;
/// Lengths and free numbers are kept within ±this (as the modifiers crate clamps them).
const BIG: f64 = 1.0e6;
/// The largest seed.
const SEED_MAX: f64 = 4_294_967_295.0;

/// The kinds the panel and menus add, with their labels.
pub const KINDS: &[(&str, &str)] = &[
    ("linearArray", "Linear Array"),
    ("circularArray", "Circular Array"),
    ("gridArray", "Grid Array"),
    ("mirror", "Mirror"),
    ("wallpaper", "Wallpaper"),
    ("copyToShape", "Copy to Shape"),
    ("transformEach", "Transform Each"),
    ("randomize", "Randomize"),
    ("stencil", "Stencil"),
    ("resample", "Resample"),
    ("trim", "Trim"),
    ("tidy", "Tidy"),
    ("deform", "Deform"),
    ("displace", "Displace"),
    ("screen", "Screen"),
];

/// The presets the menus add after the kinds, as `(kind, preset, label)`: `modifier.add` with
/// `preset`.
pub const PRESETS: &[(&str, &str, &str)] = &[("screen", "benDay", "Ben-Day Dots"), ("screen", "lineFill", "Line Fill")];

/// How a field is edited.
#[derive(Clone, Copy)]
enum F {
    /// A length in points, shown in the document's unit, within `min..=max`.
    Len(f64, f64),
    /// Degrees within `min..=max`.
    Deg(f64, f64),
    /// A factor (1 = 100%) shown as a percentage, the factor within `min..=max`.
    Pct(f64, f64),
    /// A whole number within `min..=max`.
    Int(f64, f64),
    /// A plain number within `min..=max`, two decimals.
    Num(f64, f64),
    /// A percentage stored as one (0..100), within `min..=max`.
    Percent(f64, f64),
    Bool,
    /// One of `(value, label)`.
    Choice(&'static [(&'static str, &'static str)]),
    /// Another object of the document (copy to shape).
    Target,
    /// An image object of the document (a screen's tone).
    Image,
}

const AXES: &[(&str, &str)] = &[("vertical", "Vertical axis"), ("horizontal", "Horizontal axis"), ("both", "Both axes")];
const COPY_MODES: &[(&str, &str)] = &[("points", "On its anchor points"), ("alongPath", "Along its path"), ("populate", "Scattered inside it")];
const GROUPS: &[(&str, &str)] = &[
    ("p1", "p1"),
    ("p2", "p2"),
    ("pm", "pm"),
    ("pg", "pg"),
    ("cm", "cm"),
    ("pmm", "pmm"),
    ("pmg", "pmg"),
    ("pgg", "pgg"),
    ("cmm", "cmm"),
    ("p4", "p4"),
    ("p4m", "p4m"),
    ("p4g", "p4g"),
    ("p3", "p3"),
    ("p3m1", "p3m1"),
    ("p31m", "p31m"),
    ("p6", "p6"),
    ("p6m", "p6m"),
];

const RESAMPLE_MODES: &[(&str, &str)] = &[("subdivide", "Subdivide"), ("count", "By count"), ("spacing", "By spacing")];
const SUBDIVIDE_METHODS: &[(&str, &str)] = &[("linear", "Linear"), ("catmullRom", "Catmull-Rom"), ("chaikin", "Chaikin")];
const CLOSE_METHODS: &[(&str, &str)] = &[("off", "Leave open"), ("straight", "Straight"), ("smooth", "Smooth")];
const SORTS: &[(&str, &str)] = &[
    ("none", "Keep order"),
    ("angle", "Angle"),
    ("distance", "Distance from centre"),
    ("x", "X position"),
    ("y", "Y position"),
    ("random", "Random"),
];
const DEFORM_MODES: &[(&str, &str)] = &[("taper", "Taper"), ("twist", "Twist"), ("bend", "Bend"), ("shear", "Shear"), ("bulge", "Bulge/Pinch")];
const DEFORM_AXES: &[(&str, &str)] = &[("horizontal", "Horizontal"), ("vertical", "Vertical")];
const FALLOFFS: &[(&str, &str)] = &[("linear", "Linear"), ("easeIn", "Ease in"), ("easeOut", "Ease out"), ("smooth", "Smooth")];
const SHEAR_ORIGINS: &[(&str, &str)] =
    &[("center", "Centre"), ("topLeft", "Top left"), ("bottomLeft", "Bottom left"), ("topRight", "Top right"), ("bottomRight", "Bottom right")];
const DISPLACE_MODES: &[(&str, &str)] = &[("spatial", "Spatial"), ("radial", "Radial")];
const SPOTS: &[(&str, &str)] =
    &[("round", "Round"), ("ellipse", "Ellipse"), ("line", "Line"), ("square", "Square"), ("diamond", "Diamond"), ("cross", "Cross")];
const TONES: &[(&str, &str)] = &[("fill", "Fill colour"), ("fixed", "Fixed"), ("image", "Image")];
/// A fraction of a path's length, shown as a percentage.
const FRACTION: F = F::Pct(0.0, 1.0);

const LEN: F = F::Len(-BIG, BIG);
const DEG: F = F::Deg(-BIG, BIG);
const SCALE: F = F::Pct(0.0, 100.0);
const COUNT: F = F::Int(1.0, 1000.0);
const SEED: F = F::Int(0.0, SEED_MAX);

/// Each kind's fields: `(param, label, how)`, with picabia-craft's ranges.
fn fields(kind: &str) -> &'static [(&'static str, &'static str, F)] {
    match kind {
        "linearArray" => &[
            ("count", "Copies", COUNT),
            ("offsetX", "Offset X", LEN),
            ("offsetY", "Offset Y", LEN),
            ("rotateEach", "Rotate each", DEG),
            ("scaleEach", "Scale each", SCALE),
        ],
        "circularArray" => &[
            ("count", "Copies", COUNT),
            ("radius", "Radius", F::Len(0.0, BIG)),
            ("startAngle", "Start angle", DEG),
            ("sweep", "Sweep", F::Deg(-360.0, 360.0)),
            ("centerX", "Centre X", LEN),
            ("centerY", "Centre Y", LEN),
            ("align", "Turn copies", F::Bool),
        ],
        "gridArray" => &[("rows", "Rows", COUNT), ("cols", "Columns", COUNT), ("gapX", "Gap X", LEN), ("gapY", "Gap Y", LEN)],
        "mirror" => &[("axis", "Axis", F::Choice(AXES)), ("offset", "Offset", LEN), ("keepOriginal", "Keep original", F::Bool)],
        "wallpaper" => &[
            ("group", "Group", F::Choice(GROUPS)),
            ("cols", "Columns", F::Int(1.0, 100.0)),
            ("rows", "Rows", F::Int(1.0, 100.0)),
            ("cellWidth", "Cell width", F::Len(0.001, BIG)),
            ("cellHeight", "Cell height", F::Len(0.001, BIG)),
            ("angle", "Angle", F::Deg(1.0, 179.0)),
            ("originX", "Origin X", LEN),
            ("originY", "Origin Y", LEN),
        ],
        "copyToShape" => &[
            ("target", "Target", F::Target),
            ("mode", "Place", F::Choice(COPY_MODES)),
            ("count", "Copies", COUNT),
            ("scale", "Scale", SCALE),
            ("align", "Turn copies", F::Bool),
            ("seed", "Seed", SEED),
        ],
        "transformEach" => &[
            ("offsetX", "Offset X", LEN),
            ("offsetY", "Offset Y", LEN),
            ("rotate", "Rotate", DEG),
            ("scaleX", "Scale X", SCALE),
            ("scaleY", "Scale Y", SCALE),
            ("progressive", "Progressive", F::Bool),
            // A fraction, shown as a percentage (as picabia-craft's `pct("jitter")`).
            ("jitter", "Jitter", F::Pct(0.0, 10.0)),
            ("jitterSeed", "Jitter seed", SEED),
        ],
        "randomize" => &[
            ("position", "Position", F::Len(0.0, BIG)),
            ("rotation", "Rotation", F::Deg(0.0, BIG)),
            ("scale", "Scale", SCALE),
            ("seed", "Seed", SEED),
        ],
        // Geometry rows: tlprocedural's ranges (`docs/picabia/modifiers.md`).
        "resample" => &[
            ("mode", "Mode", F::Choice(RESAMPLE_MODES)),
            ("method", "Method", F::Choice(SUBDIVIDE_METHODS)),
            ("iterations", "Iterations", F::Int(1.0, 5.0)),
            ("count", "Points", F::Int(2.0, 1024.0)),
            ("spacing", "Spacing", F::Len(1.0, 5000.0)),
        ],
        "trim" => &[("start", "Start", FRACTION), ("end", "End", FRACTION), ("offset", "Offset", FRACTION)],
        "tidy" => &[
            ("close", "Close", F::Choice(CLOSE_METHODS)),
            ("tolerance", "Join within", F::Len(0.0, 500.0)),
            ("sort", "Order", F::Choice(SORTS)),
            ("reverse", "Reverse", F::Bool),
            ("seed", "Seed", F::Int(0.0, 999_999.0)),
        ],
        "deform" => &[
            ("mode", "Mode", F::Choice(DEFORM_MODES)),
            ("axis", "Axis", F::Choice(DEFORM_AXES)),
            ("falloff", "Falloff", F::Choice(FALLOFFS)),
            ("startScale", "Start scale", F::Pct(0.1, 3.0)),
            ("endScale", "End scale", F::Pct(0.1, 3.0)),
            ("angle", "Angle", F::Deg(-720.0, 720.0)),
            ("bendAngle", "Angle", F::Deg(-360.0, 360.0)),
            ("bendPoint", "Bend point", FRACTION),
            ("shearX", "Shear X", F::Pct(-2.0, 2.0)),
            ("shearY", "Shear Y", F::Pct(-2.0, 2.0)),
            ("origin", "Origin", F::Choice(SHEAR_ORIGINS)),
            ("strength", "Strength", F::Pct(-3.0, 3.0)),
            ("radius", "Radius", F::Pct(0.01, 2.0)),
            ("progressive", "Ramp copies", F::Bool),
            ("progressiveAmount", "Ramp amount", FRACTION),
            ("progressiveReverse", "Ramp reversed", F::Bool),
        ],
        // tlprocedural's Noise panel: its ranges, fields shown by mode and by the ramp (labelled
        // as Deform's).
        "displace" => &[
            ("mode", "Mode", F::Choice(DISPLACE_MODES)),
            ("amplitude", "Amplitude", F::Len(0.0, 100.0)),
            ("scale", "Scale", F::Num(0.01, 1.0)),
            ("octaves", "Octaves", F::Int(1.0, 6.0)),
            ("rippleCount", "Ripple count", F::Int(2.0, 12.0)),
            ("seed", "Seed", F::Int(0.0, 999_999.0)),
            ("progressive", "Ramp copies", F::Bool),
            ("progressiveAmount", "Ramp amount", FRACTION),
            ("progressiveReverse", "Ramp reversed", F::Bool),
        ],
        // Picabia's own halftone: the tone's amount or image shown by the tone source.
        "screen" => &[
            ("spot", "Spot", F::Choice(SPOTS)),
            ("lpi", "Lines/inch", F::Num(1.0, 300.0)),
            ("angle", "Angle", F::Deg(-360.0, 360.0)),
            ("quality", "Quality", F::Int(4.0, 64.0)),
            ("tone", "Tone", F::Choice(TONES)),
            ("amount", "Amount", F::Percent(0.0, 100.0)),
            ("image", "Image", F::Image),
            ("invert", "Invert", F::Bool),
        ],
        _ => &[],
    }
}

/// Whether a deform in `d`'s mode uses field `key` (tlprocedural's fields per deformer; the ramp's
/// amount and direction only with the ramp on).
fn deform_applies(d: &vectorcraft_modifiers::Deform, key: &str) -> bool {
    use vectorcraft_modifiers::DeformMode::{Bend, Bulge, Shear, Taper, Twist};
    match key {
        "axis" => matches!(d.mode, Taper | Twist | Bend),
        "falloff" => matches!(d.mode, Taper | Twist | Bulge),
        "startScale" | "endScale" => d.mode == Taper,
        "angle" => d.mode == Twist,
        "bendAngle" | "bendPoint" => d.mode == Bend,
        "shearX" | "shearY" | "origin" => d.mode == Shear,
        "strength" | "radius" => d.mode == Bulge,
        "progressiveAmount" | "progressiveReverse" => d.progressive,
        _ => true,
    }
}

/// Whether row `m` uses field `key`: a wallpaper's group decides (picabia-craft's
/// `wallpaper_applies`), a resample's mode and a tidy's close and order (tlprocedural's
/// `isVisible`).
fn applies(m: &Modifier, key: &str) -> bool {
    use vectorcraft_modifiers::{CloseMethod, ResampleMode, SortBy};
    match &m.kind {
        ModifierKind::Wallpaper(w) => match key {
            "cellHeight" => w.group.uses_cell_height(),
            "angle" => w.group.uses_angle(),
            "originX" | "originY" => w.group.uses_origin(),
            _ => true,
        },
        ModifierKind::Resample(r) => match key {
            "method" | "iterations" => r.mode == ResampleMode::Subdivide,
            "count" => r.mode == ResampleMode::Count,
            "spacing" => r.mode == ResampleMode::Spacing,
            _ => true,
        },
        ModifierKind::Tidy(t) => match key {
            "tolerance" => t.close != CloseMethod::Off,
            "seed" => t.sort == SortBy::Random,
            _ => true,
        },
        ModifierKind::Deform(d) => deform_applies(d, key),
        ModifierKind::Screen(s) => match key {
            "amount" => s.tone == vectorcraft_modifiers::ToneSource::Fixed,
            "image" => s.tone == vectorcraft_modifiers::ToneSource::Image,
            _ => true,
        },
        ModifierKind::Displace(d) => {
            let radial = d.mode == vectorcraft_modifiers::DisplaceMode::Radial;
            match key {
                "scale" | "octaves" => !radial,
                "rippleCount" => radial,
                "progressiveAmount" | "progressiveReverse" => d.progressive,
                _ => true,
            }
        }
        _ => true,
    }
}

/// The label of kind `kind`, translated.
pub fn kind_label(kind: &str) -> String {
    KINDS.iter().find(|(k, _)| *k == kind).map_or_else(|| kind.to_string(), |(_, l)| tl!(l).to_string())
}

/// How many copies row `m` makes, where that reads plainly ("×5", "3 × 4"; empty otherwise).
fn amount(m: &Modifier) -> String {
    match &m.kind {
        ModifierKind::LinearArray(p) => format!("×{}", p.count),
        ModifierKind::CircularArray(p) => format!("×{}", p.count),
        ModifierKind::GridArray(p) => format!("{} × {}", p.rows, p.cols),
        _ => String::new(),
    }
}

/// One row position of the selection's stacks.
#[derive(Clone, Debug)]
struct RowView {
    kind: String,
    /// On in every object.
    enabled: bool,
    /// The first object's modifier at this position (its values are shown).
    first: Modifier,
    /// Why the first object's copy-to-shape target can't be used, if it can't.
    target_problem: Option<String>,
}

/// What the rows show for the selection.
#[derive(Clone, Debug)]
enum View {
    /// No selected object has a stack.
    Empty,
    /// The stacks have the same kinds in the same order.
    Rows { rows: Vec<RowView>, notes: Vec<String>, copies: usize },
    /// The stacks differ.
    Mixed,
    /// An object's modifier data can't be read.
    Damaged(NodeId, String),
}

/// The view of `ids` in `doc`.
fn view(doc: &Document, ids: &[NodeId]) -> View {
    let mut stacks: Vec<(NodeId, Vec<Modifier>)> = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(n) = doc.node(*id) else { continue };
        match vectorcraft_modifiers::node_stack(n) {
            None => stacks.push((*id, Vec::new())),
            Some(Ok(s)) => stacks.push((*id, s.modifiers.clone())),
            Some(Err(e)) => return View::Damaged(*id, e.to_string()),
        }
    }
    let kinds = |m: &[Modifier]| m.iter().map(|m| m.kind.id().to_string()).collect::<Vec<_>>();
    let Some((first_id, first)) = stacks.first() else { return View::Empty };
    let want = kinds(first);
    if stacks.iter().any(|(_, s)| kinds(s) != want) {
        return View::Mixed;
    }
    if want.is_empty() {
        return View::Empty;
    }
    let rows = first
        .iter()
        .enumerate()
        .map(|(i, m)| RowView {
            kind: m.kind.id().to_string(),
            enabled: stacks.iter().all(|(_, s)| s.get(i).is_some_and(|m| m.enabled)),
            first: m.clone(),
            target_problem: match &m.kind {
                ModifierKind::CopyToShape(c) if !c.target.is_empty() => {
                    vectorcraft_modifiers::parse_target(&c.target).and_then(|t| vectorcraft_modifiers::check_target(doc, *first_id, t)).err()
                }
                ModifierKind::Screen(s) if s.tone == vectorcraft_modifiers::ToneSource::Image && !s.image.is_empty() => {
                    vectorcraft_modifiers::parse_target(&s.image).and_then(|t| vectorcraft_modifiers::check_target(doc, *first_id, t)).err()
                }
                _ => None,
            },
        })
        .collect();
    // Why the first object draws once (as the canvas does), then what its evaluation says.
    let mut notes = Vec::new();
    let mut copies = 0;
    if let Some(n) = doc.node(*first_id) {
        notes.extend(vectorcraft_render::picabia::too_big(doc, n));
        if let Some(Ok(stack)) = vectorcraft_modifiers::node_stack(n) {
            let out = vectorcraft_modifiers::evaluate_with(
                &stack,
                vectorcraft_modifiers::node_bounds_in_doc(doc, n),
                &vectorcraft_modifiers::DocTargets::new(doc, *first_id),
            );
            copies = out.instances.len();
            notes.extend(out.warnings.iter().map(|w| w.message.clone()));
        }
        // What the geometry rows said while the copies were built (a row over the anchor cap…).
        notes.extend(vectorcraft_render::picabia::notes(doc, n));
    }
    View::Rows { rows, notes, copies }
}

/// The view kept for (document, revision, selection), so a frame without an edit reads no stack.
type Kept = (u64, u64, usize, Vec<NodeId>, Arc<View>);

/// The selection's view, kept across frames until the document or the selection changes.
fn kept_view(app: &VectorcraftApp, ctx: &egui::Context) -> Option<(Vec<NodeId>, Arc<View>)> {
    let st = app.session.active()?;
    let ids: Vec<NodeId> = st.selection.subjects().iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())).collect();
    let kept: Option<Kept> = pstate(ctx, "pc-view");
    let at = Arc::as_ptr(&st.doc) as usize;
    if let Some((uid, rev, kept_at, kept_ids, v)) = kept
        && uid == st.uid
        && rev == st.revision
        && kept_at == at
        && kept_ids == ids
    {
        return Some((ids, v));
    }
    let v = Arc::new(view(&st.doc, &ids));
    set_pstate::<Option<Kept>>(ctx, "pc-view", Some((st.uid, st.revision, at, ids.clone(), v.clone())));
    Some((ids, v))
}

fn row(ui: &mut Ui, selected: bool) -> (Rect, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([pos2(r.left() + EYE_W, r.top()), pos2(r.left() + EYE_W, r.bottom())], Stroke::new(1.0, t.border));
    (r, resp)
}

fn icon_hit(ui: &mut Ui, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: &str, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    icons::paint(ui, icon, r, if resp.hovered() { t.text_strong } else { t.icon });
    resp.on_hover_text(tip)
}

fn text(ui: &Ui, pos: egui::Pos2, s: &str, strong: bool, dim: bool) {
    let t = Tokens::get(ui.ctx());
    let col = if dim {
        t.text_dim
    } else if strong {
        t.text_strong
    } else {
        t.text
    };
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, s, egui::FontId::proportional(12.5), col);
}

/// A line of warning text under the rows.
fn note(ui: &mut Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width() - EYE_W - 30.0;
    let g = ui.painter().layout(s.to_string(), egui::FontId::proportional(11.5), t.warning, width);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 8.0), Sense::hover());
    ui.painter().galley(pos2(r.left() + EYE_W + 24.0, r.center().y - g.size().y / 2.0), g, t.warning);
}

/// The selection's modifier rows (none when no selected object has modifier data). The panel's
/// own `mixed` flag (its fills and effects differ) doesn't hide them: the rows compare stacks.
pub fn rows(app: &mut VectorcraftApp, ui: &mut Ui, _n: &vectorcraft_doc::Node, _mixed: bool) {
    let Some((ids, v)) = kept_view(app, ui.ctx()) else { return };
    match &*v {
        View::Empty => {}
        View::Damaged(id, why) => damaged(app, ui, *id, why),
        View::Mixed => {
            let (r, _) = row(ui, false);
            text(ui, pos2(r.left() + EYE_W + 24.0, r.center().y), tl!("Modifiers: Mixed"), false, true);
        }
        View::Rows { rows, notes, copies } => {
            let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
            let first = ids.first().copied().unwrap_or(NodeId(0));
            let selected: Option<(u64, String)> = pstate(ui.ctx(), "pc-row");
            for (i, r) in rows.iter().enumerate() {
                let sel = selected.as_ref().is_some_and(|(n, m)| *n == first.0 && *m == r.first.id);
                modifier_row(app, ui, &doc, &ids, rows.len(), i, r, sel);
            }
            for n in notes.iter().take(4) {
                note(ui, n);
            }
            let t = Tokens::get(ui.ctx());
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
            let shown = if *copies > 0 {
                crate::i18n::trn(crate::i18n::current(), *copies as u64, "{n} copy", "{n} copies")
            } else {
                tl!("Drawn once").to_string()
            };
            text(ui, pos2(r.left() + EYE_W + 24.0, r.center().y), &shown, false, true);
            let br = Rect::from_min_size(pos2(r.right() - 74.0, r.center().y - 11.0), vec2(68.0, 22.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)));
            if widgets::flat_button(&mut child, tl!("Apply"), 64.0).on_hover_text(tl!("Turn the copies into objects")).clicked() {
                app.run("modifier.apply", json!({"ids": ids_json(&ids)})).ok();
            }
        }
    }
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

/// The row for modifier data that can't be read: what is wrong, and Remove All.
fn damaged(app: &mut VectorcraftApp, ui: &mut Ui, id: NodeId, why: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = row(ui, false);
    let g = ui.painter().layout(format!("{} {why}", tl!("Modifiers:")), egui::FontId::proportional(11.5), t.warning, r.width() - EYE_W - 110.0);
    ui.painter().galley(pos2(r.left() + EYE_W + 8.0, r.center().y - g.size().y / 2.0), g, t.warning);
    let br = Rect::from_min_size(pos2(r.right() - 94.0, r.center().y - 11.0), vec2(88.0, 22.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if widgets::flat_button(&mut child, tl!("Remove All"), 84.0).clicked() {
        app.run("modifier.remove_all", json!({"ids": [id.0]})).ok();
    }
}

/// The key of row `r`'s open state: the first object and the row's modifier id (kept through
/// moves and removals of other rows).
fn open_key(first: NodeId, r: &RowView) -> String {
    format!("pc-open-{}-{}", first.0, r.first.id)
}

/// One modifier row, and its fields when open.
#[allow(clippy::too_many_arguments)]
fn modifier_row(app: &mut VectorcraftApp, ui: &mut Ui, doc: &Document, ids: &[NodeId], len: usize, i: usize, r: &RowView, selected: bool) {
    let first = ids.first().copied().unwrap_or(NodeId(0));
    let key = open_key(first, r);
    let open: bool = pstate(ui.ctx(), &key);
    let (rect, resp) = row(ui, selected);
    let er = Rect::from_center_size(pos2(rect.left() + EYE_W / 2.0, rect.center().y), vec2(16.0, 16.0));
    if icon_hit(ui, er, ("pc-eye", i), if r.enabled { "eye" } else { "eye-off" }, tl!("Click to turn this modifier on or off")).clicked() {
        app.run("modifier.toggle", json!({"ids": ids_json(ids), "index": i, "enabled": !r.enabled})).ok();
    }
    let cr = Rect::from_center_size(pos2(rect.left() + EYE_W + 12.0, rect.center().y), vec2(12.0, 12.0));
    let chev = icon_hit(ui, cr, ("pc-chev", i), if open { "chevron-down" } else { "chevron-right" }, tl!("Show the settings"));
    if chev.clicked() || resp.double_clicked() {
        set_pstate(ui.ctx(), &key, !open);
    }
    let label = match amount(&r.first) {
        a if a.is_empty() || ids.len() > 1 => kind_label(&r.kind),
        a => format!("{}  {a}", kind_label(&r.kind)),
    };
    text(ui, pos2(rect.left() + EYE_W + 24.0, rect.center().y), &label, true, !r.enabled);
    let mr = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0));
    let more = icon_hit(ui, mr, ("pc-more", i), "ellipsis", tl!("Modifier options"));
    if resp.clicked() {
        set_pstate(ui.ctx(), "pc-row", Some((first.0, r.first.id.clone())));
    }
    let menu = egui::Popup::menu(&more).open_memory(more.clicked().then_some(egui::SetOpenCommand::Toggle));
    let menu = if resp.secondary_clicked() { menu.open_memory(Some(egui::SetOpenCommand::Bool(true))) } else { menu };
    menu.show(|ui| row_menu(app, ui, ids, i, len));
    if open {
        if r.kind == "stencil" {
            let t = Tokens::get(ui.ctx());
            egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin { left: (EYE_W + 12.0) as i8, right: 6, top: 4, bottom: 4 }).show(
                ui,
                |ui| {
                    // The stencil settings edit one object (its sheet, its results).
                    if ids.len() == 1 {
                        crate::picabia_stencil_panel::stencil_section(ui, app, first);
                    } else {
                        widgets::dim_label(ui, tl!("Select one object to edit its stencil"));
                    }
                },
            );
        } else {
            editor(app, ui, doc, ids, i, r);
        }
    }
}

/// The `⋯` menu of row `i` of stacks of `len` rows.
fn row_menu(app: &mut VectorcraftApp, ui: &mut Ui, ids: &[NodeId], i: usize, len: usize) {
    let all = ids_json(ids);
    let first = ids.first().map_or(0, |i| i.0);
    let run = |app: &mut VectorcraftApp, cmd: &str, p: Value| {
        app.run(cmd, p).ok();
    };
    if widgets::menu_item(ui, tl!("Move Up"), i > 0, false) {
        run(app, "modifier.move", json!({"ids": all, "from": i, "to": i.saturating_sub(1)}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Move Down"), i + 1 < len, false) {
        run(app, "modifier.move", json!({"ids": all, "from": i, "to": i + 1}));
        ui.close();
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Duplicate Modifier"), true, false) {
        run(app, "modifier.duplicate", json!({"ids": all, "index": i}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Copy Modifier"), true, false) {
        run(app, "modifier.copy", json!({"id": first, "index": i}));
        ui.close();
    }
    let pasted = !app.session.picabia_copied_modifiers().is_empty();
    if widgets::menu_item(ui, tl!("Paste Modifiers"), pasted, false) {
        run(app, "modifier.paste", json!({"ids": all}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Remove Modifier"), true, false) {
        run(app, "modifier.remove", json!({"ids": all, "index": i}));
        ui.close();
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Apply Up to Here"), true, false) {
        run(app, "modifier.apply", json!({"ids": all, "upTo": i}));
        ui.close();
    }
}

/// `x` within the field's range.
fn clamped(x: f64, min: f64, max: f64) -> Option<f64> {
    x.is_finite().then(|| x.clamp(min, max))
}

/// The open row's fields, laid out as the Appearance panel's effect editor.
fn editor(app: &mut VectorcraftApp, ui: &mut Ui, doc: &Document, ids: &[NodeId], i: usize, r: &RowView) {
    let t = Tokens::get(ui.ctx());
    let params = r.first.kind.params();
    let unit = app.session.general_unit();
    let first = ids.first().copied().unwrap_or(NodeId(0));
    let mut change: Option<(String, Value)> = None;
    egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin { left: (EYE_W + 12.0) as i8, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let list = fields(&r.kind);
        if list.is_empty() {
            widgets::dim_label(ui, tl!("No settings here yet"));
        }
        for (key, label, how) in list {
            let Some(v) = params.get(*key).filter(|_| applies(&r.first, key)) else { continue };
            ui.horizontal(|ui| {
                let name = ui.add_sized(vec2(84.0, 22.0), egui::Label::new(egui::RichText::new(tl!(label)).size(11.5).color(t.text)).truncate());
                ui.label_inputs_by(name.id, |ui| {
                    let fid = ("pc-field", i, *key);
                    let x = v.as_f64().unwrap_or(0.0);
                    let new = match *how {
                        F::Len(lo, hi) => widgets::num_field(ui, fid, Some(x), unit, 70.0).and_then(|n| clamped(n, lo, hi)).map(|n| json!(n)),
                        F::Deg(lo, hi) => widgets::plain_field(ui, fid, x, "°", 1, 70.0).and_then(|n| clamped(n, lo, hi)).map(|n| json!(n)),
                        F::Pct(lo, hi) => {
                            widgets::plain_field(ui, fid, x * 100.0, "%", 1, 70.0).and_then(|n| clamped(n / 100.0, lo, hi)).map(|n| json!(n))
                        }
                        F::Int(lo, hi) => widgets::plain_field(ui, fid, x, "", 0, 70.0).and_then(|n| clamped(n.round(), lo, hi)).map(|n| json!(n)),
                        F::Num(lo, hi) => widgets::plain_field(ui, fid, x, "", 2, 70.0).and_then(|n| clamped(n, lo, hi)).map(|n| json!(n)),
                        F::Percent(lo, hi) => widgets::plain_field(ui, fid, x, "%", 1, 70.0).and_then(|n| clamped(n, lo, hi)).map(|n| json!(n)),
                        F::Bool => {
                            let b = v.as_bool().unwrap_or(false);
                            widgets::check_box(ui, tl!(label), b, true).then(|| json!(!b))
                        }
                        F::Choice(options) => {
                            let cur = v.as_str().unwrap_or_default();
                            let shown = options.iter().find(|(value, _)| *value == cur).map_or(cur, |(_, l)| tl!(l));
                            let labels: Vec<&str> = options.iter().map(|(_, l)| tl!(l)).collect();
                            widgets::dropdown_names(ui, fid, shown, &labels, 120.0)
                                .and_then(|k| options.get(k))
                                .filter(|(value, _)| *value != cur)
                                .map(|(value, _)| json!(value))
                        }
                        F::Target => {
                            let choices = vectorcraft_engine::cmd::picabia_modifier::target_choices(doc, first);
                            object_picker(ui, choices, v.as_str().unwrap_or_default(), fid)
                        }
                        F::Image => {
                            let choices = vectorcraft_engine::cmd::picabia_modifier::image_choices(doc, first);
                            object_picker(ui, choices, v.as_str().unwrap_or_default(), fid)
                        }
                    };
                    if let Some(new) = new {
                        change = Some((key.to_string(), new));
                    }
                });
            });
            if matches!(how, F::Target | F::Image)
                && let Some(why) = &r.target_problem
            {
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Target: {reason}"), &[("reason", why)])).size(11.0).color(t.warning));
            }
        }
    });
    if let Some((key, v)) = change {
        app.run("modifier.set", json!({"ids": ids_json(ids), "index": i, "props": {key: v}})).ok();
    }
}

/// An object dropdown: the copy-to-shape targets an object can copy onto
/// (`cmd::picabia_modifier::target_choices`), or the images a screen can read
/// (`cmd::picabia_modifier::image_choices`).
fn object_picker(ui: &mut Ui, choices: Vec<(NodeId, String)>, current: &str, fid: impl std::hash::Hash) -> Option<Value> {
    let choices: Vec<(NodeId, String)> = choices.into_iter().take(MAX_TARGETS).collect();
    let current_id = vectorcraft_modifiers::parse_target(current).ok();
    let shown = match current_id.and_then(|t| choices.iter().find(|(c, _)| *c == t)) {
        Some((_, name)) => name.clone(),
        None if current.is_empty() => tl!("Choose an object").to_string(),
        None => tl!("Not available").to_string(),
    };
    let labels: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
    let picked = widgets::dropdown_names(ui, fid, &shown, &labels, 120.0)?;
    let (t, _) = choices.get(picked)?;
    (Some(*t) != current_id).then(|| json!(t.0.to_string()))
}

/// "Add New Effect"'s Modifiers submenu: the kinds `modifier.add` adds to the selection.
pub fn add_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.menu_button(tl!("Modifiers"), |ui| {
        for (kind, label) in KINDS {
            // The geometry rows after the rows that copy.
            if *kind == "resample" {
                ui.separator();
            }
            if ui.button(tl!(label)).clicked() {
                app.run("modifier.add", json!({"kind": kind})).ok();
                ui.close();
            }
        }
        for (kind, preset, label) in PRESETS {
            if ui.button(tl!(label)).clicked() {
                app.run("modifier.add", json!({"kind": kind, "preset": preset})).ok();
                ui.close();
            }
        }
    });
}
