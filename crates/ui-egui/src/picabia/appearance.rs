//! PICABIA: the object's modifier rows in the Appearance panel.
//!
//! The rows sit in the panel's one list, as a block between the fills and strokes and the object's
//! own effects (`panels::appearance::stack` calls [`rows`]): the stack makes the copies first,
//! then the object's fills, strokes and effects draw on each copy (`docs/picabia/modifiers.md`).
//! They are built from the panel's own row parts (eye, disclosure, link label, the `fx`-style
//! editor) and VectorCraft's widgets, with colours from `theme::Tokens`; every edit goes through
//! a `modifier.*` command, so it is one undo step and agents can do the same.
//!
//! Each row: the eye turns it on and off (`modifier.toggle`), the disclosure opens its fields
//! (`modifier.set`), the label names the kind and how many copies it makes, and the `⋯` menu moves,
//! duplicates, copies, removes it or applies the stack down to it. Under the rows: the warnings the
//! evaluation gives, and Apply. "Add New Effect" (`fx`) lists the kinds under Modifiers
//! ([`add_menu`]).

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_modifiers::{Modifier, ModifierKind, Stack};

use crate::panels::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

const ROW: f32 = 30.0;
/// The eye column's width, as the Appearance panel's.
const EYE_W: f32 = 26.0;
/// The most objects the target picker lists.
const MAX_TARGETS: usize = 300;

/// The kinds the panel and menus add, with their labels (the stencil's own settings come later).
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
];

/// How a field is edited.
#[derive(Clone, Copy)]
enum F {
    /// A length in points, shown in the document's unit.
    Len,
    /// Degrees.
    Deg,
    /// A scale factor (1 = 100%), shown as a percentage.
    Pct,
    /// A whole number.
    Int,
    /// A plain number.
    Num,
    Bool,
    /// One of `(value, label)`.
    Choice(&'static [(&'static str, &'static str)]),
    /// Another object of the document (copy to shape).
    Target,
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

/// Each kind's fields: `(param, label, how)`.
fn fields(kind: &str) -> &'static [(&'static str, &'static str, F)] {
    match kind {
        "linearArray" => &[
            ("count", "Copies", F::Int),
            ("offsetX", "Offset X", F::Len),
            ("offsetY", "Offset Y", F::Len),
            ("rotateEach", "Rotate each", F::Deg),
            ("scaleEach", "Scale each", F::Pct),
        ],
        "circularArray" => &[
            ("count", "Copies", F::Int),
            ("radius", "Radius", F::Len),
            ("startAngle", "Start angle", F::Deg),
            ("sweep", "Sweep", F::Deg),
            ("centerX", "Centre X", F::Len),
            ("centerY", "Centre Y", F::Len),
            ("align", "Turn copies", F::Bool),
        ],
        "gridArray" => &[("rows", "Rows", F::Int), ("cols", "Columns", F::Int), ("gapX", "Gap X", F::Len), ("gapY", "Gap Y", F::Len)],
        "mirror" => &[("axis", "Axis", F::Choice(AXES)), ("offset", "Offset", F::Len), ("keepOriginal", "Keep original", F::Bool)],
        "wallpaper" => &[
            ("group", "Group", F::Choice(GROUPS)),
            ("cols", "Columns", F::Int),
            ("rows", "Rows", F::Int),
            ("cellWidth", "Cell width", F::Len),
            ("cellHeight", "Cell height", F::Len),
            ("angle", "Angle", F::Deg),
            ("originX", "Origin X", F::Len),
            ("originY", "Origin Y", F::Len),
        ],
        "copyToShape" => &[
            ("target", "Target", F::Target),
            ("mode", "Place", F::Choice(COPY_MODES)),
            ("count", "Copies", F::Int),
            ("scale", "Scale", F::Pct),
            ("align", "Turn copies", F::Bool),
            ("seed", "Seed", F::Int),
        ],
        "transformEach" => &[
            ("offsetX", "Offset X", F::Len),
            ("offsetY", "Offset Y", F::Len),
            ("rotate", "Rotate", F::Deg),
            ("scaleX", "Scale X", F::Pct),
            ("scaleY", "Scale Y", F::Pct),
            ("progressive", "Progressive", F::Bool),
            ("jitter", "Jitter", F::Num),
            ("jitterSeed", "Jitter seed", F::Int),
        ],
        "randomize" => &[("position", "Position", F::Len), ("rotation", "Rotation", F::Deg), ("scale", "Scale", F::Pct), ("seed", "Seed", F::Int)],
        _ => &[],
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

/// The stack rows of `n` (none without modifier data). `mixed`: several objects that differ are
/// selected, and their stacks aren't listed.
pub fn rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, mixed: bool) {
    if mixed {
        return;
    }
    let Some(read) = vectorcraft_modifiers::node_stack(n) else { return };
    let stack = match read {
        Ok(s) => s,
        Err(e) => return damaged(app, ui, n.id, &e.to_string()),
    };
    if stack.is_empty() {
        return;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let out = vectorcraft_modifiers::evaluate_with(&stack, vectorcraft_modifiers::node_bounds(n), &vectorcraft_modifiers::DocTargets::new(&doc, n.id));
    let selected: Option<usize> = pstate::<Option<usize>>(ui.ctx(), "pc-row").filter(|i| *i < stack.len());
    for (i, m) in stack.modifiers.iter().enumerate() {
        modifier_row(app, ui, &doc, n.id, &stack, i, m, selected == Some(i));
    }
    // What the evaluation says, then Apply.
    let t = Tokens::get(ui.ctx());
    for w in out.warnings.iter().take(4) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        let g = ui.painter().layout(w.message.clone(), egui::FontId::proportional(11.5), t.warning, r.width() - EYE_W - 30.0);
        ui.painter().galley(pos2(r.left() + EYE_W + 24.0, r.center().y - g.size().y / 2.0), g, t.warning);
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    let copies = out.instances.len();
    let note = if copies > 0 { tl!("{} copies").replace("{}", &copies.to_string()) } else { tl!("Drawn once").to_string() };
    text(ui, pos2(r.left() + EYE_W + 24.0, r.center().y), &note, false, true);
    let br = Rect::from_min_size(pos2(r.right() - 74.0, r.center().y - 11.0), vec2(68.0, 22.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if widgets::flat_button(&mut child, tl!("Apply"), 64.0).on_hover_text(tl!("Turn the copies into objects")).clicked() {
        app.run("modifier.apply", json!({"ids": [n.id.0]})).ok();
    }
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

/// One modifier row, and its fields when open.
#[allow(clippy::too_many_arguments)]
fn modifier_row(app: &mut VectorcraftApp, ui: &mut Ui, doc: &Document, id: NodeId, stack: &Stack, i: usize, m: &Modifier, selected: bool) {
    let open_key = format!("pc-open-{i}");
    let open: bool = pstate(ui.ctx(), &open_key);
    let (r, resp) = row(ui, selected);
    let er = Rect::from_center_size(pos2(r.left() + EYE_W / 2.0, r.center().y), vec2(16.0, 16.0));
    if icon_hit(ui, er, ("pc-eye", i), if m.enabled { "eye" } else { "eye-off" }, tl!("Click to turn this modifier on or off")).clicked() {
        app.run("modifier.toggle", json!({"ids": [id.0], "index": i})).ok();
    }
    let cr = Rect::from_center_size(pos2(r.left() + EYE_W + 12.0, r.center().y), vec2(12.0, 12.0));
    let chev = icon_hit(ui, cr, ("pc-chev", i), if open { "chevron-down" } else { "chevron-right" }, tl!("Show the settings"));
    if chev.clicked() || resp.double_clicked() {
        set_pstate(ui.ctx(), &open_key, !open);
    }
    let label = match amount(m) {
        a if a.is_empty() => kind_label(m.kind.id()),
        a => format!("{}  {a}", kind_label(m.kind.id())),
    };
    text(ui, pos2(r.left() + EYE_W + 24.0, r.center().y), &label, true, !m.enabled);
    let mr = Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0));
    let more = icon_hit(ui, mr, ("pc-more", i), "ellipsis", tl!("Modifier options"));
    if resp.clicked() {
        set_pstate(ui.ctx(), "pc-row", Some(i));
    }
    let menu = egui::Popup::menu(&more).open_memory(more.clicked().then_some(egui::SetOpenCommand::Toggle));
    let menu = if resp.secondary_clicked() { menu.open_memory(Some(egui::SetOpenCommand::Bool(true))) } else { menu };
    menu.show(|ui| row_menu(app, ui, id, i, stack.len()));
    if open {
        editor(app, ui, doc, id, i, m);
    }
}

/// The `⋯` menu of row `i` of a stack of `len` rows.
fn row_menu(app: &mut VectorcraftApp, ui: &mut Ui, id: NodeId, i: usize, len: usize) {
    let ids = json!([id.0]);
    let run = |app: &mut VectorcraftApp, cmd: &str, p: Value| {
        app.run(cmd, p).ok();
    };
    if widgets::menu_item(ui, tl!("Move Up"), i > 0, false) {
        run(app, "modifier.move", json!({"ids": ids, "from": i, "to": i.saturating_sub(1)}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Move Down"), i + 1 < len, false) {
        run(app, "modifier.move", json!({"ids": ids, "from": i, "to": i + 1}));
        ui.close();
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Duplicate Modifier"), true, false) {
        run(app, "modifier.duplicate", json!({"ids": ids, "index": i}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Copy Modifier"), true, false) {
        run(app, "modifier.copy", json!({"id": id.0, "index": i}));
        ui.close();
    }
    let pasted = !app.session.picabia_copied_modifiers().is_empty();
    if widgets::menu_item(ui, tl!("Paste Modifiers"), pasted, false) {
        run(app, "modifier.paste", json!({"ids": ids}));
        ui.close();
    }
    if widgets::menu_item(ui, tl!("Remove Modifier"), true, false) {
        run(app, "modifier.remove", json!({"ids": ids, "index": i}));
        ui.close();
    }
    ui.separator();
    if widgets::menu_item(ui, tl!("Apply Up to Here"), true, false) {
        run(app, "modifier.apply", json!({"ids": ids, "upTo": i}));
        ui.close();
    }
}

/// The open row's fields, laid out as the Appearance panel's effect editor.
fn editor(app: &mut VectorcraftApp, ui: &mut Ui, doc: &Document, id: NodeId, i: usize, m: &Modifier) {
    let t = Tokens::get(ui.ctx());
    let params = m.kind.params();
    let unit = app.session.general_unit();
    let mut change: Option<(String, Value)> = None;
    egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin { left: (EYE_W + 12.0) as i8, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let list = fields(m.kind.id());
        if list.is_empty() {
            widgets::dim_label(ui, tl!("No settings here yet"));
        }
        for (key, label, how) in list {
            let Some(v) = params.get(*key) else { continue };
            ui.horizontal(|ui| {
                let name = ui.add_sized(vec2(84.0, 22.0), egui::Label::new(egui::RichText::new(tl!(label)).size(11.5).color(t.text)).truncate());
                ui.label_inputs_by(name.id, |ui| {
                    let fid = ("pc-field", i, *key);
                    let x = v.as_f64().unwrap_or(0.0);
                    let new = match how {
                        F::Len => widgets::num_field(ui, fid, Some(x), unit, 70.0).map(|n| json!(n)),
                        F::Deg => widgets::plain_field(ui, fid, x, "°", 1, 70.0).map(|n| json!(n)),
                        F::Pct => widgets::plain_field(ui, fid, x * 100.0, "%", 1, 70.0).map(|n| json!(n / 100.0)),
                        F::Int => widgets::plain_field(ui, fid, x, "", 0, 70.0).map(|n| json!(n.round())),
                        F::Num => widgets::plain_field(ui, fid, x, "", 2, 70.0).map(|n| json!(n)),
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
                        F::Target => target_picker(ui, doc, id, v.as_str().unwrap_or_default(), fid),
                    };
                    if let Some(new) = new {
                        change = Some((key.to_string(), new));
                    }
                });
            });
        }
    });
    if let Some((key, v)) = change {
        app.run("modifier.set", json!({"ids": [id.0], "index": i, "props": {key: v}})).ok();
    }
}

/// The copy-to-shape target dropdown: the objects `id` can copy onto
/// (`cmd::picabia_modifier::target_choices`), or what is wrong with the current one.
fn target_picker(ui: &mut Ui, doc: &Document, id: NodeId, current: &str, fid: impl std::hash::Hash) -> Option<Value> {
    let choices: Vec<(NodeId, String)> = vectorcraft_engine::cmd::picabia_modifier::target_choices(doc, id).into_iter().take(MAX_TARGETS).collect();
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
            if ui.button(tl!(label)).clicked() {
                app.run("modifier.add", json!({"kind": kind})).ok();
                ui.close();
            }
        }
    });
}
