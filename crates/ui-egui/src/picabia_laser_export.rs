//! PICABIA: the laser export dialogs: a stencil's Export for Laser (`stencilExport`, asked for by
//! `modifier.stencilExport` and `modifier.stencilGcode` run without a path), the Laser Test Card
//! (`stencilTestCard`, from `modifier.stencilTestCard`) and the G-code export of any design
//! (`laserGcode`, from `laser.gcode`). One [`DialogSpec`](crate::dialogs::DialogSpec) row serves
//! the three kinds (seam in `dialogs.rs`); the commands' `{dialog: …}` reply opens it
//! ([`open_requested`], seam in `run_now`).
//!
//! The dialog holds the settings only, as fields `ui.dialog.set` can set: `format` (`svg`: SVG for
//! LightBurn and the like, `gcode`: G-code for a GRBL laser), `kind` (the test card's `card` or
//! `powerLadder`), `material`, `kerfMm`, `bedWidthMm`, `bedHeightMm`, `noLimit`, `overlapMm`,
//! `label`, the three operation colours (SVG) and the operation table `gcode` (`{cut, score,
//! engrave}`: speed, power, passes; remembered per material by the command). What the kerf, the
//! material and the table show comes from the command until they're edited (`__kerfSet`,
//! `__materialSet`, `__opsSet`). Its summary comes from the command itself, run with `dryRun: true`
//! on open and a moment after each change: sheets, their sizes in mm, lines and time, and what the
//! job warns of. Export runs the command again with the path the save panel picked (the browser:
//! with `data: true`, each sheet a download).

use egui::Ui;
use serde_json::{Map, Value, json};

use crate::dialogs::DialogSpec;
use crate::i18n;
use crate::picabia_laser_window::ops::{self, Ops};
use crate::picabia_ui::{num, readable, wrapped};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// Export for laser: a stencil's cut, one file per sheet.
pub const EXPORT: &str = "stencilExport";
/// Laser test card: a card of bridges, holes, slits, corners and a kerf gauge (or, as G-code, the
/// power ladder).
pub const TEST_CARD: &str = "stencilTestCard";
/// G-code for laser of any objects (`laser.gcode`).
pub const DESIGN: &str = "laserGcode";

/// Seconds the summary waits after a change before it asks the command again.
const DEBOUNCE: f64 = 0.3;
/// The label column of the dialog's rows.
const LABEL_W: f32 = 84.0;
const FIELD_W: f32 = 90.0;

/// The dialogs' spec (one `registry!` row for the three kinds).
pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading,
    body,
    confirm,
    ok: Some("Export…"),
    ok_enabled: Some(can_export),
    min_width: 400.0,
    max_width: Some(480.0),
    ..DialogSpec::FORM
};

fn heading(d: &Dialog) -> String {
    match d.kind.as_str() {
        TEST_CARD => tl!("Laser Test Card"),
        DESIGN => tl!("Export G-code for Laser"),
        _ => tl!("Export for Laser"),
    }
    .into()
}

/// Open the dialog a command's reply `v` asks for (`{dialog: "stencilExport" | "stencilTestCard"
/// | "laserGcode", id?, ids?, format?, kind?, material?, params?, gcode?}`); anything else is left
/// alone. Only the laser export commands' replies open it.
pub fn open_requested(app: &mut VectorcraftApp, id: &str, v: &Value) {
    let kind = match v.get("dialog").and_then(Value::as_str) {
        Some(k @ (EXPORT | TEST_CARD | DESIGN)) => k,
        _ => return,
    };
    if !matches!(id, "modifier.stencilExport" | "modifier.stencilGcode" | "modifier.stencilTestCard" | "laser.gcode") {
        return;
    }
    app.ui.dialog = Some(from_request(kind, v));
}

/// The dialog for request `data`: its fields from the settings the command would use.
pub fn from_request(kind: &str, data: &Value) -> Dialog {
    let gcode = kind == DESIGN || data.get("format").and_then(Value::as_str) == Some("gcode");
    let p = data.get("params").filter(|p| p.is_object()).unwrap_or(&Value::Null);
    let num_in = |k: &str, lo: f64, hi: f64, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v.clamp(lo, hi));
    let colour = |k: &str, d: &str| p.get(k).and_then(Value::as_str).and_then(parse_hex).map_or_else(|| d.to_string(), hex);
    let (bw, bh) = (num_in("bedWidthMm", 0.0, 5000.0, 600.0), num_in("bedHeightMm", 0.0, 5000.0, 400.0));
    let no_limit = p.get("bedWidthMm").is_some() && (bw <= 0.0 || bh <= 0.0);
    let material = data.get("material").and_then(Value::as_str).filter(|m| ops::MATERIALS.iter().any(|(id, _)| id == m)).unwrap_or("paper");
    let mut table = Ops::default();
    if let Some(g) = data.get("gcode") {
        table.read(g);
    }
    let ids: Vec<u64> = data.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).take(10_000).collect();
    let fields = json!({
        "id": data.get("id").and_then(Value::as_u64),
        "ids": ids,
        "format": if gcode { "gcode" } else { "svg" },
        "kind": if data.get("kind").and_then(Value::as_str) == Some("powerLadder") && gcode { "powerLadder" } else { "card" },
        "material": material,
        "kerfMm": num_in("kerfMm", 0.0, 2.0, 0.1),
        "bedWidthMm": if bw > 0.0 { bw.max(10.0) } else { 600.0 },
        "bedHeightMm": if bh > 0.0 { bh.max(10.0) } else { 400.0 },
        "noLimit": no_limit,
        "overlapMm": num_in("overlapMm", 0.0, 200.0, 10.0),
        "label": p.get("label").and_then(Value::as_bool).unwrap_or(true),
        "cutColour": colour("cutColour", "#ff0000"),
        "engraveColour": colour("engraveColour", "#000000"),
        "scoreColour": colour("scoreColour", "#0000ff"),
        "gcode": table.to_json(),
    });
    Dialog::new(kind, fields)
}

/// The dialog's format is G-code.
fn is_gcode(d: &Dialog) -> bool {
    d.kind == DESIGN || d.str("format") == "gcode"
}

/// The test card as G-code makes the power ladder.
fn is_ladder(d: &Dialog) -> bool {
    d.kind == TEST_CARD && is_gcode(d) && d.str("kind") == "powerLadder"
}

/// The command the dialog runs for its format.
pub fn command(d: &Dialog) -> &'static str {
    match d.kind.as_str() {
        TEST_CARD => "modifier.stencilTestCard",
        DESIGN => "laser.gcode",
        _ if is_gcode(d) => "modifier.stencilGcode",
        _ => "modifier.stencilExport",
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The command's params for the dialog's settings (without `path`, `data` or `dryRun`).
pub fn params(d: &Dialog) -> Value {
    let mut p = Map::new();
    let gcode = is_gcode(d);
    match d.kind.as_str() {
        DESIGN => {
            p.insert("ids".into(), d.fields.get("ids").cloned().unwrap_or_else(|| json!([])));
        }
        _ => {
            if let Some(id) = d.fields.get("id").and_then(Value::as_u64) {
                p.insert("id".into(), json!(id));
            }
        }
    }
    if d.kind == TEST_CARD {
        p.insert("format".into(), json!(if gcode { "gcode" } else { "svg" }));
        if gcode {
            p.insert("kind".into(), json!(if is_ladder(d) { "powerLadder" } else { "card" }));
        }
    }
    if d.kind == TEST_CARD || (gcode && d.bool("__materialSet")) {
        p.insert("material".into(), json!(d.str("material")));
    }
    if d.bool("__kerfSet") && !is_ladder(d) && d.kind != DESIGN {
        p.insert("kerfMm".into(), json!(round3(d.f64("kerfMm", 0.1).clamp(0.0, 2.0))));
    }
    if d.kind == EXPORT {
        let (w, h) = if d.bool("noLimit") {
            (0.0, 0.0)
        } else {
            (d.f64("bedWidthMm", 600.0).clamp(10.0, 5000.0), d.f64("bedHeightMm", 400.0).clamp(10.0, 5000.0))
        };
        p.insert("bedWidthMm".into(), json!(round3(w)));
        p.insert("bedHeightMm".into(), json!(round3(h)));
        p.insert("overlapMm".into(), json!(round3(d.f64("overlapMm", 10.0).clamp(0.0, 200.0))));
        p.insert("label".into(), json!(d.fields.get("label").and_then(Value::as_bool).unwrap_or(true)));
    }
    let mut v = Value::Object(p);
    if gcode {
        if d.bool("__opsSet") {
            let mut table = Ops::default();
            if let Some(g) = d.fields.get("gcode") {
                table.read(g);
            }
            table.put(&mut v);
        }
    } else if let Some(o) = v.as_object_mut() {
        for k in ["cutColour", "engraveColour", "scoreColour"] {
            if let Some(c) = d.fields.get(k).and_then(Value::as_str).and_then(parse_hex) {
                o.insert(k.into(), json!(hex(c)));
            }
        }
    }
    v
}

/// The file's extension.
fn extension(d: &Dialog) -> &'static str {
    if is_gcode(d) { "gcode" } else { "svg" }
}

/// The suggested file name: the object's name, or the card's material.
fn file_name(app: &VectorcraftApp, d: &Dialog) -> String {
    let ext = extension(d);
    let doc = app.session.active().map(|s| &s.doc);
    let named = |id: Option<u64>| {
        id.and_then(|id| doc?.node(vectorcraft_doc::NodeId(id)))
            .map(|n| n.name.as_deref().unwrap_or_default().trim().to_string())
            .filter(|n| !n.is_empty())
    };
    let first = d.fields.get("ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_u64);
    let material = d.str("material");
    match d.kind.as_str() {
        DESIGN => format!("{} laser.{ext}", named(first).unwrap_or_else(|| "Design".into())),
        TEST_CARD if is_ladder(d) => format!("Power ladder {material}.{ext}"),
        TEST_CARD => format!("Test card {material}.{ext}"),
        _ => format!("{} laser.{ext}", named(d.fields.get("id").and_then(Value::as_u64)).unwrap_or_else(|| "Stencil".into())),
    }
}

/// `#rrggbb` (or `rrggbb`) as bytes.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let byte = |i: usize| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `c` as `#rrggbb`, lowercase.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// A dry run's result, read: what the kerf, the material and the table show until edited.
fn take_result(d: &mut Dialog, r: &Result<Value, String>) {
    let Ok(v) = r else { return };
    if !d.bool("__kerfSet")
        && let Some(k) = v.get("params").and_then(|p| p.get("kerfMm")).and_then(Value::as_f64).filter(|k| k.is_finite())
    {
        d.fields.insert("kerfMm".into(), json!(k.clamp(0.0, 2.0)));
    }
    if is_gcode(d) {
        if !d.bool("__opsSet")
            && let Some(g) = v.get("gcode")
        {
            let mut table = Ops::default();
            if let Some(cur) = d.fields.get("gcode") {
                table.read(cur);
            }
            table.read(g);
            d.fields.insert("gcode".into(), table.to_json());
        }
        if !d.bool("__materialSet")
            && let Some(m) = v.get("material").and_then(Value::as_str).filter(|m| ops::MATERIALS.iter().any(|(id, _)| id == m))
        {
            d.fields.insert("material".into(), json!(m));
        }
    }
}

/// Ask the command what these settings come to, when they changed a moment ago.
fn refresh(app: &mut VectorcraftApp, ctx: &egui::Context, d: &mut Dialog) {
    let p = params(d);
    let ran = json!({"cmd": command(d), "params": p});
    if d.fields.get("__ran") == Some(&ran) {
        return;
    }
    let now = ctx.input(|i| i.time);
    let wait = d.f64("__changedAt", 0.0) + DEBOUNCE - now;
    if d.fields.contains_key("__ran") && wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        return;
    }
    let mut dry = p;
    // Only the sheets' sizes and counts: nothing written, and the remembered settings stay.
    dry["dryRun"] = json!(true);
    // Read-only and not undoable: run directly so a refusal shows in the dialog.
    let r = app.session.execute(command(d), &dry).map_err(|e| i18n::msg(&e.to_string()));
    take_result(d, &r);
    let summary = match r {
        Ok(v) => json!({"files": v.get("files").cloned().unwrap_or(Value::Null), "warnings": v.get("warnings").cloned().unwrap_or(Value::Null)}),
        Err(e) => json!({"error": e}),
    };
    d.fields.insert("__summary".into(), summary);
    // The params as they are now (the result may have filled the kerf or the table in).
    d.fields.insert("__ran".into(), json!({"cmd": command(d), "params": params(d)}));
}

/// OK can export: the last dry run didn't refuse.
fn can_export(_app: &VectorcraftApp, d: &Dialog) -> bool {
    d.fields.get("__summary").is_some_and(|s| s.get("error").is_none())
}

fn lead(ui: &mut Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui(egui::vec2(LABEL_W, 24.0), |ui| {
        ui.set_min_width(LABEL_W);
        ui.label(egui::RichText::new(label).size(12.0).color(t.text));
    });
}

/// A mm field of `d`: whether it changed.
fn mm_field(ui: &mut Ui, d: &mut Dialog, key: &str, name: &str, lo: f64, hi: f64, default: f64) -> bool {
    let v = d.f64(key, default);
    match widgets::plain_field_named(ui, ("picabia_laser_export", key), Some(name), v, "mm", 2, FIELD_W) {
        Some(n) if n.is_finite() => {
            d.fields.insert(key.into(), json!(n.clamp(lo, hi)));
            true
        }
        _ => false,
    }
}

/// The settings rows; whether one changed.
fn fields(ui: &mut Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    if d.kind != DESIGN {
        ui.horizontal(|ui| {
            lead(ui, tl!("Format"));
            let gcode = is_gcode(d);
            if widgets::radio(ui, "SVG for LightBurn", !gcode, true) && gcode {
                d.fields.insert("format".into(), json!("svg"));
                // The ladder is G-code only.
                d.fields.insert("kind".into(), json!("card"));
                changed = true;
            }
            if widgets::radio(ui, "G-code for GRBL", gcode, true) && !gcode {
                d.fields.insert("format".into(), json!("gcode"));
                changed = true;
            }
        });
    }
    let gcode = is_gcode(d);
    if d.kind == TEST_CARD && gcode {
        ui.horizontal(|ui| {
            lead(ui, tl!("Make"));
            let ladder = is_ladder(d);
            if widgets::radio(ui, "Test card", !ladder, true) && ladder {
                d.fields.insert("kind".into(), json!("card"));
                changed = true;
            }
            if widgets::radio(ui, "Power ladder", ladder, true) && !ladder {
                d.fields.insert("kind".into(), json!("powerLadder"));
                changed = true;
            }
        });
    }
    if d.kind == TEST_CARD || gcode {
        ui.horizontal(|ui| {
            lead(ui, tl!("Material"));
            if let Some(m) = ops::material_dropdown(ui, "export", &d.str("material"), FIELD_W * 2.0) {
                d.fields.insert("material".into(), json!(m));
                d.fields.insert("__materialSet".into(), json!(true));
                // The card's kerf follows the material until one is typed, and the operations the
                // material's remembered ones until edited.
                if d.kind == TEST_CARD {
                    d.fields.insert("__kerfSet".into(), json!(false));
                }
                d.fields.insert("__opsSet".into(), json!(false));
                changed = true;
            }
        });
    }
    if d.kind != DESIGN && !is_ladder(d) {
        ui.horizontal(|ui| {
            lead(ui, tl!("Kerf"));
            if mm_field(ui, d, "kerfMm", tl!("Kerf"), 0.0, 2.0, 0.1) {
                d.fields.insert("__kerfSet".into(), json!(true));
                changed = true;
            }
        });
    }
    if d.kind == EXPORT {
        ui.horizontal(|ui| {
            lead(ui, tl!("Bed"));
            if d.bool("noLimit") {
                crate::picabia_ui::note(ui, tl!("No limit"));
            } else {
                changed |= mm_field(ui, d, "bedWidthMm", tl!("Bed width"), 10.0, 5000.0, 600.0);
                crate::picabia_ui::note(ui, "×");
                changed |= mm_field(ui, d, "bedHeightMm", tl!("Bed height"), 10.0, 5000.0, 400.0);
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            let on = d.bool("noLimit");
            if widgets::check(ui, "No limit", on, true) {
                d.fields.insert("noLimit".into(), json!(!on));
                changed = true;
            }
        });
        if !d.bool("noLimit") {
            ui.horizontal(|ui| {
                lead(ui, tl!("Overlap"));
                changed |= mm_field(ui, d, "overlapMm", tl!("Overlap"), 0.0, 200.0, 10.0);
            });
        }
    }
    ui.add_space(4.0);
    if gcode {
        let mut table = Ops::default();
        if let Some(g) = d.fields.get("gcode") {
            table.read(g);
        }
        if ops::table(ui, "export", &mut table) {
            d.fields.insert("gcode".into(), table.to_json());
            d.fields.insert("__opsSet".into(), json!(true));
            changed = true;
        }
    } else {
        for (key, label) in [("cutColour", tl!("Cut")), ("engraveColour", tl!("Engrave")), ("scoreColour", tl!("Score"))] {
            ui.horizontal(|ui| {
                lead(ui, label);
                let mut c = d.fields.get(key).and_then(Value::as_str).and_then(parse_hex).unwrap_or([0, 0, 0]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                    d.fields.insert(key.into(), json!(hex(c)));
                    changed = true;
                }
                if let Some(rgb) = widgets::hex_field_named(ui, ("picabia_laser_export", key), Some(label), &hex(c)).as_deref().and_then(parse_hex)
                    && rgb != c
                {
                    d.fields.insert(key.into(), json!(hex(rgb)));
                    changed = true;
                }
            });
        }
    }
    if d.kind == EXPORT {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            let on = d.fields.get("label").and_then(Value::as_bool).unwrap_or(true);
            if widgets::check(ui, "Engrave a label", on, true) {
                d.fields.insert("label".into(), json!(!on));
                changed = true;
            }
        });
    }
    let _ = t;
    changed
}

/// The summary: the sheets, their sizes and what the job warns of.
fn summary(ui: &mut Ui, d: &Dialog) {
    let t = Tokens::get(ui.ctx());
    let Some(s) = d.fields.get("__summary") else {
        crate::picabia_ui::note(ui, tl!("Working out the sheets…"));
        return;
    };
    if let Some(e) = s.get("error").and_then(Value::as_str) {
        wrapped(ui, e, t.error);
        return;
    }
    let files: Vec<&Value> = s.get("files").and_then(Value::as_array).into_iter().flatten().take(64).collect();
    ui.label(egui::RichText::new(i18n::tn(files.len() as u64, "{n} sheet", "{n} sheets")).font(crate::theme::semibold(12.0)).color(t.text));
    for f in &files {
        let size = i18n::fmt(
            tl!("{w} × {h} mm"),
            &[("w", &num(crate::picabia_ui::f(f, "wMm").unwrap_or(0.0))), ("h", &num(crate::picabia_ui::f(f, "hMm").unwrap_or(0.0)))],
        );
        let cuts = match (f.get("lines").and_then(Value::as_u64), crate::picabia_ui::f(f, "estimatedSec")) {
            (Some(n), Some(e)) => format!(
                "{}, {}",
                i18n::tn(n, "{n} line", "{n} lines"),
                i18n::fmt(tl!("about {time}"), &[("time", &crate::picabia_laser_window::status::clock(e))])
            ),
            (Some(n), None) => i18n::tn(n, "{n} line", "{n} lines"),
            _ => i18n::tn(f.get("contours").and_then(Value::as_u64).unwrap_or(0), "{n} cut", "{n} cuts"),
        };
        let name = f.get("sheet").or_else(|| f.get("name")).and_then(Value::as_str).unwrap_or_default();
        // A lone sheet needs no "1 of 1".
        let line = if name.is_empty() || files.len() == 1 { format!("{size}, {cuts}") } else { format!("{name}: {size}, {cuts}") };
        crate::picabia_ui::note(ui, &line);
        let loose = f.get("loosePieces").and_then(Value::as_u64).unwrap_or(0);
        if loose > 0 {
            wrapped(ui, &i18n::tn(loose, "{n} piece would fall out", "{n} pieces would fall out"), readable(t.warning, &t));
        }
    }
    for w in crate::picabia_ui::strings(s, "warnings", 32) {
        wrapped(ui, &i18n::msg(&w), readable(t.warning, &t));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let doc = app.session.active().map(|s| s.doc.clone());
    let first = d
        .fields
        .get("id")
        .and_then(Value::as_u64)
        .or_else(|| d.fields.get("ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_u64));
    if let Some(name) = first.and_then(|id| doc.as_ref()?.node(vectorcraft_doc::NodeId(id)).and_then(|n| n.name.clone())).filter(|n| !n.is_empty()) {
        let more = d.fields.get("ids").and_then(Value::as_array).map_or(0, Vec::len).saturating_sub(1);
        let name = if more > 0 { i18n::fmt(&i18n::tn(more as u64, "{name} and {n} more", "{name} and {n} more"), &[("name", &name)]) } else { name };
        ui.label(egui::RichText::new(name).size(12.0).color(t.text_dim));
        ui.add_space(4.0);
    }
    if fields(ui, d) {
        d.fields.insert("__changedAt".into(), json!(ui.ctx().input(|i| i.time)));
    }
    refresh(app, ui.ctx(), d);
    ui.add_space(8.0);
    widgets::divider(ui);
    summary(ui, d);
    false
}

/// "Saved Rose laser.svg" or "Saved 3 files: Rose laser 1.svg, …".
pub fn saved_text(names: &[String]) -> String {
    match names {
        [one] => i18n::fmt(tl!("Saved {name}"), &[("name", one)]),
        _ => i18n::fmt(&i18n::tn(names.len() as u64, "Saved {n} file: {names}", "Saved {n} files: {names}"), &[("names", &names.join(", "))]),
    }
}

/// The file names a command result wrote.
pub fn written(r: &Value) -> Vec<String> {
    r.get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let p = f.get("path").or_else(|| f.get("name")).and_then(Value::as_str)?;
            Some(std::path::Path::new(p).file_name().map_or(p.to_string(), |n| n.to_string_lossy().into_owned()))
        })
        .collect()
}

/// Export: a save panel for the first sheet's file (more sheets take " 1", " 2"... beside it); in
/// the browser each sheet downloads.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let cmd = command(d);
    let mut p = params(d);
    let ext = extension(d);
    if crate::io::is_web(app) {
        p["data"] = json!(true);
        let r = app.run(cmd, p)?;
        let mut names = Vec::new();
        for f in r.get("files").and_then(Value::as_array).into_iter().flatten() {
            let name = f.get("name").and_then(Value::as_str).map_or_else(|| format!("laser.{ext}"), str::to_string);
            let bytes = f.get("data").and_then(Value::as_str).and_then(vectorcraft_format::base64_decode).ok_or("no data")?;
            crate::io::write_named(app, None, &name, &bytes)?;
            names.push(name);
        }
        app.status(saved_text(&names));
        app.ui.dialog = None;
        app.prefs_changed = true;
        return Ok(json!({"files": names}));
    }
    let folder =
        app.session.active().and_then(|s| s.path.as_deref()).and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned());
    let label = if ext == "gcode" { "G-code" } else { "SVG" };
    let pick = crate::FilePick {
        name: file_name(app, d),
        folder: folder.filter(|f| !f.is_empty()),
        filters: crate::FilePick::filters([(label, &[ext][..])]),
    };
    let mut path = crate::picks::save(app, &pick).ok_or("cancelled")?;
    if std::path::Path::new(&path).extension().is_none_or(|e| e.is_empty()) {
        path.push('.');
        path.push_str(ext);
    }
    p["path"] = json!(path);
    let r = app.run(cmd, p)?;
    app.status(saved_text(&written(&r)));
    app.ui.dialog = None;
    // The command remembered these settings: they're saved with the preferences.
    app.prefs_changed = true;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        app
    }

    /// A stencil of two squares → its id.
    fn stencil(app: &mut VectorcraftApp) -> u64 {
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.ellipse", json!({"x": 30, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [b], "color": "#ffffff"})).unwrap();
        app.run("paint.setFill", json!({"ids": [a], "color": "#000000"})).unwrap();
        app.run("modifier.stencilFromSelection", json!({"ids": [a, b], "props": {"widthMm": 120}})).unwrap()["id"].as_u64().unwrap()
    }

    fn painted(app: &mut VectorcraftApp) -> String {
        let ctx = crate::widgets::look_tests::themed(crate::theme::Brightness::Light);
        let mut text = String::new();
        // Twice: the first frame lays the window out.
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
            text.clear();
            for s in out.shapes {
                if let egui::Shape::Text(t) = s.shape {
                    text.push_str(t.galley.text());
                    text.push('\n');
                }
            }
        }
        text
    }

    #[test]
    fn hex_reads_and_writes() {
        assert_eq!(parse_hex("#FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("#ff80"), None);
        assert_eq!(parse_hex("#ééé"), None);
        assert_eq!(hex([1, 2, 255]), "#0102ff");
    }

    #[test]
    fn junk_requests_open_with_defaults() {
        for junk in
            [json!({}), json!({"params": 5, "gcode": "x", "ids": "y", "material": "wood"}), json!({"params": {"kerfMm": "a", "bedWidthMm": 1e300}})]
        {
            let d = from_request(EXPORT, &junk);
            assert_eq!((d.str("format"), d.str("material")), ("svg".into(), "paper".into()));
            let p = params(&d);
            assert!(p["bedWidthMm"].as_f64().unwrap() <= 5000.0);
        }
        // A bed of 0 × 0: no limit.
        let d = from_request(EXPORT, &json!({"params": {"bedWidthMm": 0, "bedHeightMm": 0}}));
        assert!(d.bool("noLimit"));
        assert_eq!(params(&d)["bedWidthMm"], 0.0);
    }

    #[test]
    fn running_the_export_without_a_path_opens_the_dialog_and_its_dry_run_fills_it() {
        let mut app = app();
        let g = stencil(&mut app);
        app.run("modifier.stencilExport", json!({"id": g})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.fields["id"].as_u64()), (EXPORT, Some(g)));
        assert_eq!(command(&d), "modifier.stencilExport");
        let text = painted(&mut app);
        for want in ["Export for Laser", "Format", "SVG for LightBurn", "G-code for GRBL", "Kerf", "Bed", "1 sheet", "Export…"] {
            assert!(text.contains(want), "{want} in {text}");
        }
        // G-code: the stencil's G-code command, the operation table and its lines.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("format".into(), json!("gcode"));
        // Changed a while ago: the summary asks again now.
        d.fields.insert("__changedAt".into(), json!(-1.0));
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(command(&d), "modifier.stencilGcode");
        let text = painted(&mut app);
        for want in ["Speed", "Power", "Passes", "Material", "lines"] {
            assert!(text.contains(want), "{want} in {text}");
        }
    }

    #[test]
    fn exporting_writes_the_file_the_save_panel_picks() {
        let mut app = app();
        let g = stencil(&mut app);
        let dir = std::env::temp_dir().join(format!("picabia-b6-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Stencil laser.gcode");
        let picked = path.to_string_lossy().into_owned();
        app.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
        app.run("modifier.stencilGcode", json!({"id": g})).unwrap();
        let r = crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "closed");
        assert_eq!(written(&r), ["Stencil laser.gcode"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("M5") && text.contains("G21"), "{text}");
        assert!(app.prefs_changed, "the remembered settings are saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_test_card_and_design_dialogs_open_from_their_commands() {
        let mut app = app();
        app.run("modifier.stencilTestCard", json!({"format": "gcode", "kind": "powerLadder"})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(d.kind, TEST_CARD);
        assert!(is_ladder(&d));
        assert_eq!(params(&d)["kind"], "powerLadder");
        let text = painted(&mut app);
        assert!(text.contains("Laser Test Card") && text.contains("Power ladder"), "{text}");
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.ui.dialog = None;
        app.run("laser.gcode", json!({"ids": [id]})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), command(&d)), (DESIGN, "laser.gcode"));
        assert_eq!(params(&d)["ids"], json!([id]));
        let text = painted(&mut app);
        assert!(text.contains("Export G-code for Laser"), "{text}");
        // Other replies that name a dialog are left alone.
        app.ui.dialog = None;
        open_requested(&mut app, "file.new", &json!({"dialog": EXPORT}));
        assert!(app.ui.dialog.is_none());
    }
}
