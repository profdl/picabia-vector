//! PICABIA: a stencil's settings and what it came to, for one object: [`stencil_section`] (which
//! the Appearance panel's stencil row calls) and the standalone Stencil window around it
//! (Window ▸ Stencil Settings…, `ui.stencilSettings`), plus the stencil's markers on the canvas.
//!
//! The section reads the object's stack with `modifier.list` and the computed stencil from the
//! engine's store (never computing it on the UI thread: the background refresh does), and every
//! edit is one `modifier.set` of the stencil row (one undo step). Its groups: Colours (cut the
//! lighter colours or the ones picked, from the design's own colours), Size (width, margin, the
//! sheet), Clean-up (material preset and limits), Ties (tie bars, a tie pattern or gauze), Repeat
//! (follow a wallpaper or grid row above, one tile), Registration holes, Preview (paper or dyed
//! cloth and their colours); then the stats line, one row per kind of problem (each click shows
//! the next one on the canvas) and Export for Laser… / Test Card… / Laser….

use std::sync::Arc;

use egui::{Color32, Id, Pos2, Shape, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::kurbo::{Point, Rect};

use crate::VectorcraftApp;
use crate::i18n;
use crate::picabia_ui::{button, num, readable, wrapped};
use crate::theme::{Tokens, semibold};
use crate::widgets;

/// The label column of the section's rows.
const LABEL_W: f32 = 104.0;
const FIELD_W: f32 = 84.0;
/// The Stencil window's width.
const WINDOW_W: f32 = 340.0;

/// A choice: (value written in the param, label).
type Choices = &'static [(&'static str, &'static str)];

/// Colour modes (`colourMode`). `separate` isn't offered.
const MODES: Choices = &[("threshold", "Cut the lighter colours"), ("pick", "Cut the colours I pick")];
/// Previews (`preview`).
const PREVIEWS: Choices = &[("paper", "Paper preview"), ("cloth", "Dyed cloth preview")];
/// How islands are held (`tieMode`).
const TIE_MODES: Choices = &[("ties", "Tie the islands"), ("pattern", "Tie with a pattern"), ("gauze", "Hold the islands with gauze")];
/// Tie lattices (`patternType`).
const PATTERN_TYPES: Choices = &[("grid", "Grid"), ("horizontal", "Horizontal lines"), ("vertical", "Vertical lines")];
/// Where a lattice goes (`patternWhere`).
const PATTERN_WHERE: Choices = &[("everywhere", "Everywhere"), ("islands", "Only around islands")];
/// Sheet materials (`material`): a preset fills the cutting limits.
const MATERIALS: Choices = &[("paper", "Cut from paper"), ("film", "Cut from film"), ("card", "Cut from card"), ("custom", "Custom limits")];
/// What is cut (`output`).
const OUTPUTS: Choices = &[("sheet", "The whole sheet"), ("tile", "One repeat tile")];
/// Whether the repeat above is followed (`repeat`).
const REPEATS: Choices = &[("auto", "Follow the repeat above"), ("off", "Ignore the repeat")];
/// What clean-up does with what breaks a limit (`limitMode`).
const LIMIT_MODES: Choices = &[("fix", "Fix what's too fine"), ("flag", "Only flag what's too fine")];

/// The problem kinds, in the order the rows list them: (kind as `modifier.stencilInfo` names it,
/// its row's words for one and for several).
const PROBLEMS: &[(&str, &str, &str)] = &[
    ("narrowPaper", "{n} narrow paper bridge", "{n} narrow paper bridges"),
    ("narrowCut", "{n} narrow cut", "{n} narrow cuts"),
    ("smallCut", "{n} small cut", "{n} small cuts"),
    ("smallHole", "{n} small hole", "{n} small holes"),
    ("sharpCorner", "{n} sharp corner", "{n} sharp corners"),
    ("untied", "{n} island has no tie and would fall out", "{n} islands have no tie and would fall out"),
];

/// The stencil row of an object's stack, as `modifier.list` gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// The row's id (`modifier.set`'s `modifier`).
    pub id: String,
    pub params: serde_json::Map<String, Value>,
    /// An enabled `wallpaper` or `gridArray` row is above it (the repeat it can follow).
    pub lattice_above: bool,
    /// PIC6: what its bound fields resolved to (`modifier.list`'s `bound`), and the document's
    /// parameters for the binding chips (set by [`stencil_section`]).
    pub bound: Vec<vectorcraft_modifiers::BoundField>,
    pub doc_params: vectorcraft_modifiers::Parameters,
    pub scope: Arc<vectorcraft_modifiers::Scope>,
}

/// The object's first enabled stencil row (`list` is `modifier.list`'s reply).
pub fn stencil_row(list: &Value) -> Option<Row> {
    let rows = list.get("modifiers").and_then(Value::as_array)?;
    let mut lattice_above = false;
    for r in rows.iter().take(256) {
        let enabled = r.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let kind = r.get("kind").and_then(Value::as_str).unwrap_or_default();
        if enabled && kind == "stencil" {
            let id = r.get("id").and_then(Value::as_str)?.to_string();
            let bound = list.get("bound").and_then(Value::as_array).map(|b| bound_fields(b, &id)).unwrap_or_default();
            return Some(Row { id, params: r.as_object().cloned().unwrap_or_default(), lattice_above, bound, ..Row::default() });
        }
        if enabled && matches!(kind, "wallpaper" | "gridArray") {
            lattice_above = true;
        }
    }
    None
}

/// Row `id`'s entries of `modifier.list`'s `bound`.
fn bound_fields(list: &[Value], id: &str) -> Vec<vectorcraft_modifiers::BoundField> {
    list.iter()
        .take(256)
        .filter(|b| b.get("modifier").and_then(Value::as_str) == Some(id))
        .filter_map(|b| {
            Some(vectorcraft_modifiers::BoundField {
                modifier: id.to_string(),
                field: b.get("field")?.as_str()?.to_string(),
                value: b.get("value").and_then(Value::as_f64),
                per_copy: b.get("perCopy").and_then(Value::as_bool).unwrap_or(false),
                error: b.get("error").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

impl Row {
    /// Field `k`'s binding (the row's `bind`).
    fn binding(&self, k: &str) -> Option<vectorcraft_modifiers::Binding> {
        self.params.get("bind").and_then(|b| b.get(k)).map(vectorcraft_modifiers::Binding::from_value)
    }

    /// Field `k`'s bound value as text (`fmt` of the value used), when it is bound.
    fn bound_text(&self, k: &str, own: f64, fmt: &dyn Fn(f64) -> String) -> Option<(String, bool)> {
        let binding = self.binding(k);
        let bound = self.bound.iter().find(|b| b.field == k);
        let text = crate::picabia::bind_chip::shown_value(binding.as_ref(), bound, own, fmt)?;
        let failed = bound.is_some_and(|b| b.error.is_some()) || matches!(binding, Some(vectorcraft_modifiers::Binding::Other(_)));
        Some((text, failed))
    }

    /// Field `k`'s binding chip (labelled `label`); what was asked goes into `e`.
    fn chip(&self, ui: &mut Ui, k: &str, label: &str, e: &mut Edits) {
        let binding = self.binding(k);
        let c = crate::picabia::bind_chip::Chip {
            id: ui.scope_id().with(("picabia_stencil_chip", k)),
            label,
            binding: binding.as_ref(),
            bound: self.bound.iter().find(|b| b.field == k),
            per_copy_field: false,
            params: &self.doc_params,
            scope: &self.scope,
        };
        if let Some(a) = crate::picabia::bind_chip::chip(ui, &c) {
            e.binds.push((k.to_string(), a));
        }
    }

    fn str(&self, k: &str, d: &'static str) -> String {
        self.params.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
    }
    fn num(&self, k: &str, d: f64) -> f64 {
        self.params.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d)
    }
    fn on(&self, k: &str, d: bool) -> bool {
        self.params.get(k).and_then(Value::as_bool).unwrap_or(d)
    }
}

/// What the computed stencil came to (`modifier.stencilInfo`'s shape), read leniently.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// The design's colours: (hex, cut).
    pub colours: Vec<(String, bool)>,
    pub warnings: Vec<String>,
    pub stats: Value,
    /// The marked places, document space: (kind, bounds).
    pub problems: Vec<(String, Rect)>,
    /// Registration holes' centres.
    pub hoshi: Vec<Point>,
    pub sheet_mm: Option<(f64, f64)>,
    pub tile_mm: Option<(f64, f64)>,
    /// The result is from what the document holds now (else it's being worked out again).
    pub fresh: bool,
}

impl Info {
    /// Read `modifier.stencilInfo`'s JSON.
    pub fn from_json(v: &Value, fresh: bool) -> Info {
        use crate::picabia_ui::f;
        let rect = |p: &Value| Some(Rect::new(f(p, "x")?, f(p, "y")?, f(p, "x")? + f(p, "w")?, f(p, "y")? + f(p, "h")?));
        let size = |k: &str| v.get(k).and_then(|s| Some((f(s, "w")?, f(s, "h")?)));
        Info {
            colours: v
                .get("colours")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(256)
                .filter_map(|c| Some((c.get("hex")?.as_str()?.to_string(), c.get("cut").and_then(Value::as_bool).unwrap_or(false))))
                .collect(),
            warnings: crate::picabia_ui::strings(v, "warnings", 32),
            stats: v.get("stats").cloned().unwrap_or(Value::Null),
            problems: v
                .get("problems")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(500)
                .filter_map(|p| Some((p.get("kind")?.as_str()?.to_string(), rect(p)?)))
                .collect(),
            hoshi: v
                .get("hoshi")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(64)
                .filter_map(|p| Some(Point::new(f(p, "x")?, f(p, "y")?)))
                .collect(),
            sheet_mm: size("sheetMm"),
            tile_mm: size("tile"),
            fresh,
        }
    }

    fn stat(&self, k: &str) -> u64 {
        self.stats.get(k).and_then(Value::as_u64).unwrap_or(0)
    }

    /// How many problems of `kind` there are (all of them, past the marked ones too).
    fn count(&self, kind: &str) -> u64 {
        self.stats.get("problems").and_then(|p| p.get(kind)).and_then(Value::as_u64).unwrap_or(0)
    }
}

/// Object `node`'s computed stencil, as the engine keeps it (never computed here).
pub fn info_of(app: &VectorcraftApp, node: NodeId) -> Option<Info> {
    let doc = &app.session.active()?.doc;
    let n = doc.node(node)?;
    let (sheet, fresh) = vectorcraft_engine::picabia_stencil::lookup(doc, n)?;
    Some(Info::from_json(&vectorcraft_engine::picabia_stencil::info_json(&sheet.info), fresh))
}

/// The stats line: "3 pieces, 2 islands tied with 4 ties, 1 speck dropped, 812.5 mm cut".
pub fn stats_line(info: &Info, gauze: bool) -> String {
    let pieces = i18n::tn(info.stat("cutPieces"), "{n} piece", "{n} pieces");
    let specks = i18n::tn(info.stat("droppedSpecks"), "{n} speck dropped", "{n} specks dropped");
    let length = num(info.stats.get("cutLengthMm").and_then(Value::as_f64).unwrap_or(0.0));
    let islands = info.stat("islands");
    if islands == 0 {
        return i18n::fmt(tl!("{pieces}, {specks}, {length} mm cut"), &[("pieces", &pieces), ("specks", &specks), ("length", &length)]);
    }
    let held = if gauze {
        i18n::tn(islands, "{n} island held by the gauze", "{n} islands held by the gauze")
    } else {
        let pattern = info.stat("patternHeld");
        let tied = islands.saturating_sub(pattern).saturating_sub(info.stat("untied"));
        let ties = i18n::tn(info.stat("ties"), "{n} tie", "{n} ties");
        let by_ties = i18n::fmt(&i18n::tn(tied, "{n} island tied with {ties}", "{n} islands tied with {ties}"), &[("ties", &ties)]);
        match pattern {
            0 => by_ties,
            p if tied == 0 => i18n::tn(p, "{n} island held by the pattern", "{n} islands held by the pattern"),
            p => format!("{}, {by_ties}", i18n::tn(p, "{n} island held by the pattern", "{n} islands held by the pattern")),
        }
    };
    i18n::fmt(
        tl!("{pieces}, {islands}, {specks}, {length} mm cut"),
        &[("pieces", &pieces), ("islands", &held), ("specks", &specks), ("length", &length)],
    )
}

/// The problem rows a stencil shows: (kind, its words for one, for several, count), counts above
/// 0 only; with gauze no island falls out, so untied ones aren't problems.
pub fn problem_list(info: &Info, gauze: bool) -> Vec<(&'static str, &'static str, &'static str, u64)> {
    PROBLEMS
        .iter()
        .map(|(k, one, other)| {
            let n = match *k {
                "untied" if gauze => 0,
                "untied" => info.count(k).max(info.stat("untied")),
                _ => info.count(k),
            };
            (*k, *one, *other, n)
        })
        .filter(|(.., n)| *n > 0)
        .collect()
}

/// The problem marker the canvas emphasises (the last one a problem row showed). UI state, kept
/// in egui's memory, never in the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Focus {
    pub node: NodeId,
    pub kind: String,
    pub index: usize,
}

fn focus_id() -> Id {
    Id::unique("picabia_stencil_problem_focus")
}

/// The focused problem marker, if a problem row was clicked.
pub fn focus(ctx: &egui::Context) -> Option<Focus> {
    ctx.data(|d| d.get_temp::<Focus>(focus_id()))
}

/// One edit of the stencil row: `modifier.set` of `props` (one undo step).
fn set(app: &mut VectorcraftApp, node: NodeId, row: &Row, props: Value) {
    let _ = app.run("modifier.set", json!({"ids": [node.0], "modifier": row.id, "props": props}));
}

fn lead(ui: &mut Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui(vec2(LABEL_W, 24.0), |ui| {
        ui.set_min_width(LABEL_W);
        ui.label(egui::RichText::new(label).size(12.0).color(t.text));
    });
}

/// A group's small heading.
fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).size(11.0).color(t.text_dim));
}

/// A choice row: the new value when one was picked.
fn choice(ui: &mut Ui, row: &Row, key: &str, label: &str, options: Choices, default: &'static str) -> Option<String> {
    let current = row.str(key, default);
    let shown = options.iter().find(|(v, _)| *v == current).map_or(current.as_str(), |(_, l)| *l);
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let mut out = None;
    ui.horizontal(|ui| {
        lead(ui, label);
        if let Some(i) = widgets::dropdown_named(ui, ("picabia_stencil", key), Some(label), shown, &labels, 196.0) {
            out = options.get(i).map(|(v, _)| (*v).to_string()).filter(|v| *v != current);
        }
    });
    out
}

/// A number row in `suffix` units (clamped to `lo..=hi`): the new value when one was committed.
/// A bound field shows the value it takes, read-only; every number row has a binding chip.
#[allow(clippy::too_many_arguments)]
fn number(ui: &mut Ui, row: &Row, e: &mut Edits, key: &str, label: &str, suffix: &str, lo: f64, hi: f64, default: f64) -> Option<f64> {
    let mut out = None;
    ui.horizontal(|ui| {
        lead(ui, label);
        let own = row.num(key, default);
        if let Some((text, failed)) = row.bound_text(key, own, &|v| format!("{} {suffix}", num(v))) {
            crate::picabia::bind_chip::bound_value(ui, &text, failed, FIELD_W);
        } else if let Some(v) = widgets::plain_field_named(ui, ("picabia_stencil", key), Some(label), own, suffix, 2, FIELD_W)
            && v.is_finite()
        {
            out = Some(v.clamp(lo, hi));
        }
        row.chip(ui, key, label, e);
    });
    out
}

/// A check row: the new value when it was clicked.
fn check(ui: &mut Ui, row: &Row, key: &str, label: &str, default: bool) -> Option<bool> {
    let on = row.on(key, default);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
        if widgets::check(ui, label, on, true) {
            out = Some(!on);
        }
    });
    out
}

/// A colour row (`#rrggbb`): the new colour when one was picked or typed.
fn colour(ui: &mut Ui, row: &Row, key: &str, label: &str, default: &'static str) -> Option<String> {
    let current = row.str(key, default);
    let mut c = crate::picabia_laser_export::parse_hex(&current).unwrap_or([0, 0, 0]);
    let mut out = None;
    ui.horizontal(|ui| {
        lead(ui, label);
        if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
            out = Some(crate::picabia_laser_export::hex(c));
        }
        if let Some(rgb) = widgets::hex_field_named(ui, ("picabia_stencil", key), Some(label), &crate::picabia_laser_export::hex(c))
            .as_deref()
            .and_then(crate::picabia_laser_export::parse_hex)
        {
            out = Some(crate::picabia_laser_export::hex(rgb));
        }
    });
    out.filter(|c| *c != current.to_ascii_lowercase())
}

/// The design's colours, each with whether it's cut (`pick` mode): the new `cutColours` when one
/// was ticked or unticked.
fn colour_list(ui: &mut Ui, row: &Row, info: Option<&Info>) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    let Some(info) = info else {
        crate::picabia_ui::note(ui, tl!("The design's colours show here once the stencil is drawn"));
        return None;
    };
    if info.colours.is_empty() {
        crate::picabia_ui::note(ui, tl!("The design has no colours the stencil can cut"));
        return None;
    }
    let picked: Vec<String> = match row.params.get("cutColours") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect(),
        Some(Value::String(s)) => vec![s.to_ascii_lowercase()],
        _ => Vec::new(),
    };
    let mut out = None;
    egui::ScrollArea::vertical().id_salt("picabia_stencil_colours").max_height(160.0).show(ui, |ui| {
        for (hex, _) in &info.colours {
            let on = picked.contains(&hex.to_ascii_lowercase());
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                let c = crate::picabia_laser_export::parse_hex(hex).map_or(Color32::BLACK, |[r, g, b]| Color32::from_rgb(r, g, b));
                ui.painter().rect_filled(r, 3.0, c);
                ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                if widgets::check_name(ui, hex, on) {
                    let mut next: Vec<String> = picked.iter().filter(|p| **p != hex.to_ascii_lowercase()).cloned().collect();
                    if !on {
                        next.push(hex.to_ascii_lowercase());
                    }
                    out = Some(json!(next));
                }
            });
        }
    });
    out
}

/// Why the Tile output can't be cut, when the stack has no repeat for it.
fn tile_problem(row: &Row) -> Option<&'static str> {
    if !row.lattice_above {
        Some(tl!("A tile needs a Wallpaper or Grid array row above the stencil"))
    } else if row.str("repeat", "auto") == "off" {
        Some(tl!("A tile needs the stencil to follow the repeat above"))
    } else {
        None
    }
}

/// The edits a frame of the section asks for: each one `modifier.set` of the stencil row.
#[derive(Default)]
struct Edits {
    sets: Vec<Value>,
    /// What the binding chips asked: (field, action), each one `modifier.bind` or
    /// `modifier.unbind` (PIC6).
    binds: Vec<(String, crate::picabia::bind_chip::ChipAction)>,
}

impl Edits {
    fn push(&mut self, k: &str, v: Value) {
        self.sets.push(json!({ k: v }));
    }
}

/// What [`stencil_section`] reads: `modifier.list` and the stencil's [`Info`].
type Section = (Value, Option<Info>);
/// [`Section`] kept by document (uid, revision, address), stencil store generation and object.
type KeptSection = ((u64, u64, usize, u64, u64), Arc<Section>);

/// `node`'s section data, read again only when the document, its stencil results or the object
/// change (not every frame while the row is open).
fn kept_section(app: &mut VectorcraftApp, ctx: &egui::Context, node: NodeId) -> Arc<Section> {
    let key = app
        .session
        .active()
        .map(|st| (st.uid, st.revision, Arc::as_ptr(&st.doc) as usize, vectorcraft_engine::picabia_stencil::generation(), node.0));
    if let Some(key) = key
        && let Some((k, v)) = crate::panels::pstate::<Option<KeptSection>>(ctx, "picabia-stencil-section")
        && k == key
    {
        return v;
    }
    let list = app.session.execute("modifier.list", &json!({"id": node.0})).unwrap_or(Value::Null);
    let v = Arc::new((list, info_of(app, node)));
    if let Some(key) = key {
        crate::panels::set_pstate::<Option<KeptSection>>(ctx, "picabia-stencil-section", Some((key, v.clone())));
    }
    v
}

/// Object `node`'s stencil settings and what it came to (the Appearance panel's stencil row and
/// the Stencil window call this). An object without a stencil row offers to make one.
pub fn stencil_section(ui: &mut Ui, app: &mut VectorcraftApp, node: NodeId) {
    let t = Tokens::get(ui.ctx());
    let kept = kept_section(app, ui.ctx(), node);
    let (list, info) = (&kept.0, &kept.1);
    let Some(mut row) = stencil_row(list) else {
        wrapped(ui, tl!("This object isn't a stencil. Make Stencil turns the selection into a laser-cut stencil sheet."), t.text_dim);
        ui.add_space(4.0);
        if button(ui, "Make Stencil", true, true) {
            let _ = app.run("modifier.stencilFromSelection", json!({"ids": [node.0]}));
        }
        return;
    };
    if let Some(st) = app.session.active() {
        row.doc_params = vectorcraft_modifiers::Parameters::of_doc(&st.doc);
        row.scope = vectorcraft_modifiers::Scope::of_doc(&st.doc);
    }
    let mut e = Edits::default();
    colours_group(ui, &row, info.as_ref(), &mut e);
    size_group(ui, &row, info.as_ref(), &mut e);
    cleanup_group(ui, &row, &mut e);
    ties_group(ui, &row, &mut e);
    repeat_group(ui, &row, &mut e);
    preview_group(ui, &row, &mut e);
    ui.add_space(8.0);
    widgets::divider(ui);
    let gauze = row.str("tieMode", "ties") == "gauze";
    match &info {
        None => crate::picabia_ui::note(ui, tl!("Working out the stencil…")),
        Some(info) => {
            let line = stats_line(info, gauze);
            ui.label(egui::RichText::new(line).size(11.5).color(t.text));
            if !info.fresh {
                crate::picabia_ui::note(ui, tl!("Updating…"));
            }
            problem_rows(app, ui, node, info, gauze);
            for w in &info.warnings {
                wrapped(ui, &i18n::msg(w), readable(t.warning, &t));
            }
        }
    }
    ui.add_space(6.0);
    let (mut export, mut card, mut laser) = (false, false, false);
    ui.horizontal_wrapped(|ui| {
        export = button(ui, "Export for Laser…", false, true);
        card = button(ui, "Test Card…", false, true);
        laser = button(ui, "Laser…", false, true);
    });
    for edit in e.sets {
        set(app, node, &row, edit);
    }
    for (field, a) in e.binds {
        crate::picabia::appearance::run_chip(app, json!({"ids": [node.0], "modifier": row.id, "field": field}), a);
    }
    if export {
        let _ = app.run("modifier.stencilExport", json!({"id": node.0}));
    }
    if card {
        let _ = app.run("modifier.stencilTestCard", json!({"id": node.0}));
    }
    if laser {
        let _ = app.run(crate::picabia_ui::LASER_PANEL, json!({}));
    }
}

/// Colours: what is cut (the lighter colours, or the ones picked) and Invert.
fn colours_group(ui: &mut Ui, row: &Row, info: Option<&Info>, e: &mut Edits) {
    heading(ui, tl!("Colours"));
    if let Some(v) = choice(ui, row, "colourMode", tl!("Cut"), MODES, "threshold") {
        e.push("colourMode", json!(v));
    }
    if row.str("colourMode", "threshold") == "pick" {
        if let Some(v) = colour_list(ui, row, info) {
            e.push("cutColours", v);
        }
    } else {
        // Stored 0 to 1, shown as a percentage.
        ui.horizontal(|ui| {
            lead(ui, tl!("Threshold"));
            let own = row.num("threshold", 0.5);
            if let Some((text, failed)) = row.bound_text("threshold", own, &|v| format!("{:.1}%", v * 100.0)) {
                crate::picabia::bind_chip::bound_value(ui, &text, failed, FIELD_W);
            } else if let Some(v) =
                widgets::plain_field_named(ui, ("picabia_stencil", "threshold"), Some(tl!("Threshold")), own * 100.0, "%", 1, FIELD_W)
                && v.is_finite()
            {
                e.push("threshold", json!(v.clamp(0.0, 100.0) / 100.0));
            }
            row.chip(ui, "threshold", tl!("Threshold"), e);
        });
    }
    if let Some(v) = check(ui, row, "invert", tl!("Invert (cut the other colours)"), false) {
        e.push("invert", json!(v));
    }
}

/// Size: the design's width, the margin and the sheet it makes.
fn size_group(ui: &mut Ui, row: &Row, info: Option<&Info>, e: &mut Edits) {
    heading(ui, tl!("Size"));
    if let Some(v) = number(ui, row, e, "widthMm", tl!("Width"), "mm", 1.0, 10_000.0, 300.0) {
        e.push("widthMm", json!(v));
    }
    if let Some(v) = number(ui, row, e, "marginMm", tl!("Margin"), "mm", 0.0, 1000.0, 15.0) {
        e.push("marginMm", json!(v));
    }
    if let Some((w, h)) = info.and_then(|i| i.tile_mm.or(i.sheet_mm)) {
        ui.horizontal(|ui| {
            lead(ui, tl!("Sheet"));
            crate::picabia_ui::note(ui, &i18n::fmt(tl!("{w} × {h} mm"), &[("w", &num(w)), ("h", &num(h))]));
        });
    }
}

/// Clean-up: the material preset and the cutting limits.
fn cleanup_group(ui: &mut Ui, row: &Row, e: &mut Edits) {
    heading(ui, tl!("Clean-up"));
    if let Some(v) = choice(ui, row, "material", tl!("Material"), MATERIALS, "paper") {
        e.push("material", json!(v));
    }
    if let Some(v) = choice(ui, row, "limitMode", tl!("Limits"), LIMIT_MODES, "fix") {
        e.push("limitMode", json!(v));
    }
    for (key, label, suffix, hi, d) in [
        ("minCutAreaMm2", tl!("Smallest cut"), "mm²", 10_000.0, 0.5),
        ("minCutWidthMm", tl!("Thinnest cut"), "mm", 100.0, 0.3),
        ("minBridgeMm", tl!("Thinnest paper"), "mm", 100.0, 0.5),
        ("toleranceMm", tl!("Smoothing"), "mm", 10.0, 0.05),
        ("minCornerAngle", tl!("Sharpest corner"), "°", 179.0, 30.0),
    ] {
        if let Some(v) = number(ui, row, e, key, label, suffix, 0.0, hi, d) {
            e.push(key, json!(v));
        }
    }
}

/// Ties: how the islands are held (tie bars, a tie pattern, gauze) and the ties' settings.
fn ties_group(ui: &mut Ui, row: &Row, e: &mut Edits) {
    heading(ui, tl!("Ties"));
    let tie_mode = row.str("tieMode", "ties");
    if let Some(v) = choice(ui, row, "tieMode", tl!("Islands"), TIE_MODES, "ties") {
        e.push("tieMode", json!(v));
    }
    if tie_mode == "pattern" {
        if let Some(v) = choice(ui, row, "patternType", tl!("Pattern"), PATTERN_TYPES, "grid") {
            e.push("patternType", json!(v));
        }
        if let Some(v) = number(ui, row, e, "patternWidthMm", tl!("Tie size"), "mm", 0.05, 50.0, 0.6) {
            e.push("patternWidthMm", json!(v));
        }
        if let Some(v) = number(ui, row, e, "patternSpacingMm", tl!("Spacing"), "mm", 0.5, 500.0, 8.0) {
            e.push("patternSpacingMm", json!(v));
        }
        if let Some(v) = number(ui, row, e, "patternAngle", tl!("Angle"), "°", 0.0, 360.0, 0.0) {
            e.push("patternAngle", json!(v));
        }
        if let Some(v) = choice(ui, row, "patternWhere", tl!("Where"), PATTERN_WHERE, "everywhere") {
            e.push("patternWhere", json!(v));
        }
    }
    if tie_mode != "gauze" {
        if let Some(v) = number(ui, row, e, "tieWidthMm", tl!("Tie width"), "mm", 0.05, 50.0, 1.0) {
            e.push("tieWidthMm", json!(v));
        }
        if let Some(v) = number(ui, row, e, "tiesPerIsland", tl!("Ties per island"), "", 1.0, 8.0, 2.0) {
            e.push("tiesPerIsland", json!(v.round()));
        }
        ui.horizontal(|ui| {
            lead(ui, tl!("Direction"));
            let angle = row.params.get("tieAngle").and_then(Value::as_f64).filter(|a| a.is_finite());
            match angle {
                Some(a) => {
                    if let Some(v) = widgets::plain_field_named(ui, ("picabia_stencil", "tieAngle"), Some(tl!("Direction")), a, "°", 1, FIELD_W)
                        && v.is_finite()
                    {
                        e.push("tieAngle", json!(v.rem_euclid(360.0)));
                    }
                    if button(ui, "Any", false, true) {
                        e.push("tieAngle", Value::Null);
                    }
                }
                None => {
                    crate::picabia_ui::note(ui, tl!("Any"));
                    if button(ui, "Choose", false, true) {
                        e.push("tieAngle", json!(0.0));
                    }
                }
            }
        });
        if let Some(v) = number(ui, row, e, "tieSeed", tl!("Seed"), "", 0.0, 1e9, 1.0) {
            e.push("tieSeed", json!(v.round()));
        }
        if let Some(v) = check(ui, row, "tieSpread", tl!("Spread the ties around each island"), true) {
            e.push("tieSpread", json!(v));
        }
    }
}

/// Repeat: following a wallpaper or grid row above, the sheet or one tile, registration holes.
fn repeat_group(ui: &mut Ui, row: &Row, e: &mut Edits) {
    let t = Tokens::get(ui.ctx());
    heading(ui, tl!("Repeat"));
    if row.lattice_above
        && let Some(v) = choice(ui, row, "repeat", tl!("Repeat"), REPEATS, "auto")
    {
        e.push("repeat", json!(v));
    }
    if let Some(v) = choice(ui, row, "output", tl!("Cut"), OUTPUTS, "sheet") {
        e.push("output", json!(v));
    }
    if row.str("output", "sheet") == "tile"
        && let Some(why) = tile_problem(row)
    {
        wrapped(ui, why, readable(t.warning, &t));
    }
    if let Some(v) = check(ui, row, "hoshi", tl!("Registration holes"), true) {
        e.push("hoshi", json!(v));
    }
    if row.on("hoshi", true)
        && let Some(v) = number(ui, row, e, "hoshiDiameterMm", tl!("Hole diameter"), "mm", 0.2, 20.0, 1.5)
    {
        e.push("hoshiDiameterMm", json!(v));
    }
}

/// Preview: paper or dyed cloth, and their colours.
fn preview_group(ui: &mut Ui, row: &Row, e: &mut Edits) {
    heading(ui, tl!("Preview"));
    let preview = row.str("preview", "paper");
    if let Some(v) = choice(ui, row, "preview", tl!("Show"), PREVIEWS, "paper") {
        e.push("preview", json!(v));
    }
    if preview != "cloth" {
        if let Some(v) = colour(ui, row, "paperColour", tl!("Paper"), "#c8a165") {
            e.push("paperColour", json!(v));
        }
    } else {
        if let Some(v) = colour(ui, row, "pasteColour", tl!("Paste"), "#1f3a8a") {
            e.push("pasteColour", json!(v));
        }
        if let Some(v) = colour(ui, row, "clothColour", tl!("Cloth"), "#f4efe6") {
            e.push("clothColour", json!(v));
        }
    }
}

/// One warning row per kind of problem, clickable when its places are marked: each click shows
/// the next one on the canvas ("2 of 12").
fn problem_rows(app: &mut VectorcraftApp, ui: &mut Ui, node: NodeId, info: &Info, gauze: bool) {
    let t = Tokens::get(ui.ctx());
    let focused = focus(ui.ctx()).filter(|f| f.node == node);
    for (kind, one, other, n) in problem_list(info, gauze) {
        let places: Vec<Rect> = info.problems.iter().filter(|(k, _)| k == kind).map(|(_, b)| *b).collect();
        let text = i18n::tn(n, one, other);
        let shown = focused.as_ref().filter(|f| f.kind == kind && f.index < places.len()).map(|f| f.index);
        let resp = ui
            .horizontal_wrapped(|ui| {
                let sense = if places.is_empty() { egui::Sense::hover() } else { egui::Sense::click() };
                let resp = ui.add(
                    egui::Label::new(egui::RichText::new(&text).size(11.5).color(readable(t.warning, &t))).wrap().selectable(false).sense(sense),
                );
                if let Some(i) = shown {
                    crate::picabia_ui::note(
                        ui,
                        &i18n::fmt(tl!("{n} of {total}"), &[("n", &(i + 1).to_string()), ("total", &places.len().to_string())]),
                    );
                }
                resp
            })
            .inner;
        if places.is_empty() {
            continue;
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tl!("Show the next one on the canvas"));
        if resp.clicked() {
            let index = match &focused {
                Some(f) if f.kind == kind => (f.index + 1) % places.len(),
                _ => 0,
            };
            ui.ctx().data_mut(|d| d.insert_temp(focus_id(), Focus { node, kind: kind.to_string(), index }));
            if let Some(b) = places.get(index) {
                reveal(app, *b);
            }
        }
    }
}

/// Centre the view on `b` (document space), zooming in until it's at least about 48 points across
/// on screen (at most 8 times the current zoom) and out until it fits.
fn reveal(app: &mut VectorcraftApp, b: Rect) {
    if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) {
        return;
    }
    let size = app.canvas_rect.map_or((800.0, 600.0), |r| (f64::from(r.width()), f64::from(r.height())));
    let Some(v) = app.view_mut() else { return };
    let side = b.width().max(b.height()).max(1e-6);
    let fits = ((size.0 - 128.0).max(50.0) / b.width().max(1e-6)).min((size.1 - 128.0).max(50.0) / b.height().max(1e-6));
    let legible = (48.0 / side).min(v.zoom * 8.0);
    let zoom = v.zoom.max(legible).min(fits).clamp(0.01, 640.0);
    if zoom.is_finite() {
        v.zoom = zoom;
    }
    v.center = b.center();
}

/// The Stencil window (Window ▸ Stencil Settings…): the first selected object's stencil.
pub fn window(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.picabia.stencil_open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut open = true;
    let node = crate::picabia_laser_window::job::selection(app).first().copied();
    egui::Window::new(tl!("Stencil"))
        .id(Id::unique("picabia_stencil_window"))
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().right_top() + vec2(-WINDOW_W - 320.0, 72.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(14)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(WINDOW_W);
            let max = (ctx.content_rect().height() - 180.0).max(200.0);
            egui::ScrollArea::vertical().id_salt("picabia_stencil_scroll").max_height(max).min_scrolled_height(max.min(760.0)).show(ui, |ui| {
                match node {
                    Some(node) => {
                        if let Some(name) = app.session.active().and_then(|d| d.doc.node(node)).and_then(|n| n.name.clone()).filter(|n| !n.is_empty())
                        {
                            ui.label(egui::RichText::new(name).font(semibold(12.5)).color(t.text_strong));
                        }
                        stencil_section(ui, app, node);
                    }
                    None => wrapped(ui, tl!("Select a stencil object, or the objects to make one from."), t.text_dim),
                }
            });
        });
    app.picabia.stencil_open = open;
}

/// Below this many points across on screen a problem is ringed rather than boxed.
const RING_BELOW: f32 = 14.0;
/// The most markers drawn a frame.
const MAX_MARKERS: usize = 600;

/// The selected stencils' problems and registration holes on the canvas (the focused problem
/// emphasised), over the art and under the panels and windows.
pub fn canvas_markers(app: &VectorcraftApp, ctx: &egui::Context) {
    let (Some(rect), Some(view)) = (app.canvas_rect, app.view()) else { return };
    let Some(st) = app.session.active() else { return };
    let xf = crate::canvas::Xf::at(rect, view, app.canvas_ui_zoom);
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, Id::unique("picabia_stencil_markers"))).with_clip_rect(rect);
    let focused = focus(ctx);
    let mut budget = MAX_MARKERS;
    for id in st.selection.objects.iter().take(16) {
        let Some(n) = st.doc.node(*id).filter(|n| vectorcraft_engine::picabia_stencil::is_stencil(n)) else { continue };
        let Some((sheet, _)) = vectorcraft_engine::picabia_stencil::lookup(&st.doc, n) else { continue };
        let info = &sheet.info;
        for h in info.hoshi.iter().filter(|h| h.x.is_finite() && h.y.is_finite()) {
            if budget == 0 {
                return;
            }
            budget -= 1;
            painter.circle_stroke(xf.to_screen(*h), 6.0, Stroke::new(1.0, t.selection));
        }
        let parsed = Info::from_json(&vectorcraft_engine::picabia_stencil::info_json(info), true);
        // With gauze no island falls out: its untied islands aren't problems. (Only asked when
        // there are some: it evaluates the stack.)
        let gauze = parsed.problems.iter().any(|(k, _)| k == "untied")
            && vectorcraft_engine::picabia_stencil::frame(&st.doc, n).is_some_and(|f| format!("{:?}", f.params.tie_mode) == "Gauze");
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (kind, b) in &parsed.problems {
            let k = seen.entry(kind.clone()).or_insert(0);
            let index = *k;
            *k += 1;
            if gauze && kind == "untied" {
                continue;
            }
            if budget == 0 {
                return;
            }
            budget -= 1;
            let emphasised = focused.as_ref().is_some_and(|f| f.node == *id && f.kind == *kind && f.index == index);
            marker(&painter, &xf, *b, t.warning, t.selection, emphasised);
        }
    }
}

/// One problem marker for `b` (document space): a ring around a place too small to box, else its
/// bounds dashed; the focused one thicker and haloed in the selection colour.
fn marker(painter: &egui::Painter, xf: &crate::canvas::Xf, b: Rect, warn: Color32, halo: Color32, focused: bool) {
    let c: Vec<Pos2> = xf.quad(b);
    let r = egui::Rect::from_points(&c);
    let width = if focused { 2.5 } else { 1.5 };
    if r.width().max(r.height()) < RING_BELOW {
        let radius = if focused { 10.0 } else { 6.0 };
        if focused {
            painter.circle_stroke(r.center(), radius + 3.0, Stroke::new(2.0, halo));
        }
        painter.circle_stroke(r.center(), radius, Stroke::new(width, warn));
    } else if focused {
        painter.add(Shape::closed_line(c.clone(), Stroke::new(4.5, halo)));
        painter.add(Shape::closed_line(c, Stroke::new(width, warn)));
    } else {
        let mut ring = c.clone();
        if let Some(first) = c.first() {
            ring.push(*first);
        }
        painter.extend(Shape::dashed_line(&ring, Stroke::new(width, warn), 4.0, 3.0));
    }
}

#[cfg(test)]
mod tests;
