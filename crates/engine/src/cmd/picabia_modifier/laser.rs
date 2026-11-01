//! PICABIA: laser export of stencil objects (`modifier.stencilExport`) and the laser test card
//! (`modifier.stencilTestCard`).
//!
//! The geometry is `vectorcraft_stencil`'s ([`vectorcraft_stencil::laser_job`],
//! [`vectorcraft_stencil::test_card`], [`vectorcraft_stencil::laser_svg`]); here the settings are
//! layered (the remembered ones in `Prefs::picabia_laser`, then the material's kerf, then the
//! command's params), the stencil is brought up to date, and the sheets are written to files
//! through the engine's file access (automation roots apply), or returned as base64.
//!
//! This only writes text: nothing here opens a port or talks to a machine.
//!
//! Without `path`, `data` or `dryRun` a command changes nothing and returns
//! `{dialog: "<dialog id>", …}`: what the UI's dialog opens with (the commands can't open one).

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_modifiers::{MaterialLimits, Stencil, StencilMaterial};
use vectorcraft_stencil::{LaserInput, LaserParams, LaserSheet};

use crate::{EngineError, Result, Session};

pub(super) const EXPORT: &str = "modifier.stencilExport";
pub(super) const TEST_CARD: &str = "modifier.stencilTestCard";

/// `p[key]`, a null value counting as absent.
pub(super) fn get<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}

pub(super) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    super::super::bad(cmd, msg)
}

fn disabled(cmd: &str, why: impl Into<String>) -> EngineError {
    EngineError::Disabled(cmd.into(), why.into())
}

/// Which remembered Picabia setting: the laser export's (`Prefs::picabia_laser`) or the machine
/// profile (`Prefs::picabia_machine`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Pref {
    Laser,
    Machine,
}

/// A remembered Picabia setting; null when there is none.
pub(super) fn pref(s: &Session, key: Pref) -> Value {
    match key {
        Pref::Laser => s.prefs.picabia_laser.clone(),
        Pref::Machine => s.prefs.picabia_machine.clone(),
    }
}

/// Remember a Picabia setting.
pub(super) fn set_pref(s: &mut Session, key: Pref, v: Value) {
    match key {
        Pref::Laser => s.prefs.picabia_laser = v,
        Pref::Machine => s.prefs.picabia_machine = v,
    }
}

/// Where the sheets go: a file path, base64 data, only their description (a dry run: the
/// dialog's summary), or (none of those) the dialog.
#[derive(Debug, PartialEq)]
pub(super) enum Out {
    Path(String),
    Data,
    Summary,
    Dialog,
}

/// `dryRun: true` wins, then `path` (a non-empty string), then `data: true`; none: the dialog.
pub(super) fn out_of(cmd: &str, p: &Value) -> Result<Out> {
    match get(p, "dryRun") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => return Ok(Out::Summary),
        Some(_) => return Err(bad(cmd, "`dryRun` must be true or false")),
    }
    match get(p, "path") {
        None => {}
        Some(Value::String(s)) if !s.trim().is_empty() => return Ok(Out::Path(s.clone())),
        Some(_) => return Err(bad(cmd, "`path` must be a file path")),
    }
    match get(p, "data") {
        None | Some(Value::Bool(false)) => Ok(Out::Dialog),
        Some(Value::Bool(true)) => Ok(Out::Data),
        Some(_) => Err(bad(cmd, "`data` must be true or false")),
    }
}

/// The settings a command uses: defaults, then the remembered ones, then `kerf` (the material's;
/// `None` keeps the remembered kerf), then the command's own params. Clamped.
pub(super) fn settings(s: &Session, kerf: Option<f64>, p: &Value) -> LaserParams {
    let mut lp = LaserParams::default();
    lp.apply_json(&pref(s, Pref::Laser));
    if let Some(k) = kerf {
        lp.apply_json(&json!({"kerfMm": k}));
    }
    lp.apply_json(p);
    lp
}

/// The stencil object a command acts on: `id`/`ids` or the selection, its first object.
pub(super) fn stencil_object(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let id = super::super::targets(s, p).map_err(|e| bad(cmd, e.to_string()))?.first().copied();
    let id = id.ok_or_else(|| disabled(cmd, "select a stencil object"))?;
    let n = s.doc()?.doc.node(id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
    if !crate::picabia_stencil::is_stencil(n) {
        return Err(disabled(cmd, format!("object {} isn't a stencil (its modifiers have no stencil row)", id.0)));
    }
    Ok(id)
}

/// A material's name as it is engraved and shown: "Paper", "Film", "Card", "Custom".
pub(super) fn material_name(m: StencilMaterial) -> &'static str {
    match m {
        StencilMaterial::Paper => "Paper",
        StencilMaterial::Film => "Film",
        StencilMaterial::Card => "Card",
        StencilMaterial::Custom => "Custom",
    }
}

/// A length in mm for a label: one decimal at most, without a trailing `.0`.
fn mm(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

/// `name` as a file name: path separators and characters file systems refuse become `-`.
pub(super) fn file_safe(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .take(120)
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Stencil".into() } else { s }
}

/// `path` for sheet `k` (1-based) of `n`: `path` itself for one sheet, else " k" before the
/// extension (`Rose.svg` → `Rose 1.svg`; without an extension, at the end).
pub fn sheet_path(path: &str, k: usize, n: usize) -> String {
    if n <= 1 {
        return path.to_string();
    }
    let p = std::path::Path::new(path);
    match (p.extension().and_then(std::ffi::OsStr::to_str), p.file_stem().and_then(std::ffi::OsStr::to_str)) {
        (Some(ext), Some(_)) if path.len() > ext.len() => match path.get(..path.len() - ext.len() - 1) {
            Some(base) => format!("{base} {k}.{ext}"),
            None => format!("{path} {k}"),
        },
        _ => format!("{path} {k}"),
    }
}

/// The file name part of `path`.
pub(super) fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(path).to_string()
}

/// Write `bytes` to `path` through the engine's file access (automation roots, atomic writes).
pub(super) fn write_out(path: &str, bytes: &[u8]) -> Result<()> {
    super::super::fileio::write_file(path, bytes)
}

pub(super) fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Write (or encode) each sheet as an SVG, and describe it.
fn emit(sheets: &[LaserSheet], lp: &LaserParams, out: &Out, base: &str) -> Result<Vec<Value>> {
    let n = sheets.len();
    let mut files = Vec::with_capacity(n);
    for (i, sheet) in sheets.iter().enumerate() {
        let mut f = json!({
            "sheet": sheet.name,
            "wMm": finite(sheet.w_mm),
            "hMm": finite(sheet.h_mm),
            "contours": sheet.cut.len(),
            "loosePieces": sheet.loose_pieces,
        });
        let name = sheet_path(&format!("{base}.svg"), i + 1, n);
        match out {
            Out::Path(path) => {
                let svg = vectorcraft_stencil::laser_svg(sheet, lp);
                let path = sheet_path(path, i + 1, n);
                write_out(&path, svg.as_bytes())?;
                f["name"] = json!(file_name(&path));
                f["path"] = json!(path);
            }
            Out::Data => {
                let svg = vectorcraft_stencil::laser_svg(sheet, lp);
                f["name"] = json!(name);
                f["data"] = json!(vectorcraft_format::base64_encode(svg.as_bytes()));
            }
            Out::Summary | Out::Dialog => f["name"] = json!(name),
        }
        files.push(f);
    }
    Ok(files)
}

/// A stencil object's laser sheets (for the SVG and the G-code export).
pub(super) struct StencilSheets {
    pub sheets: Vec<LaserSheet>,
    pub warnings: Vec<String>,
    /// The object's name made safe for a file name.
    pub base: String,
}

/// Stencil object `id`'s params (clamped), material and kerf (`None`: custom, keep the remembered
/// kerf).
pub(super) fn stencil_material(cmd: &str, s: &Session, id: NodeId) -> Result<(Stencil, Option<f64>)> {
    let doc = &s.doc()?.doc;
    let n = doc.node(id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
    let frame = crate::picabia_stencil::frame(doc, n).ok_or_else(|| disabled(cmd, format!("object {} has no size to cut", id.0)))?;
    let kerf = frame.params.material.limits().map(|l| l.kerf_mm);
    Ok((frame.params, kerf))
}

/// Stencil object `id`'s drawn cut (the tile with tile output) as kerf-compensated laser sheets
/// with `lp`. Errors: no size, nothing to cut, a refused job.
pub(super) fn stencil_sheets(cmd: &str, s: &Session, id: NodeId, lp: &LaserParams) -> Result<StencilSheets> {
    let doc = &s.doc()?.doc;
    let n = doc.node(id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
    let frame = crate::picabia_stencil::frame(doc, n).ok_or_else(|| disabled(cmd, format!("object {} has no size to cut", id.0)))?;
    let sheet = crate::picabia_stencil::ensure_one(doc, id)
        .ok_or_else(|| EngineError::Other(format!("{cmd}: object {}'s stencil couldn't be computed", id.0)))?;
    let info = &sheet.info;
    if sheet.cut.elements().is_empty() {
        let why = info.warnings.first().map(|w| format!(" ({w})")).unwrap_or_default();
        return Err(disabled(cmd, format!("object {}'s stencil has nothing to cut{why}", id.0)));
    }
    let size = match info.tile_mm {
        Some(_) => "Repeat tile".to_string(),
        None => format!("Sheet {} × {} mm", mm(info.sheet_mm.0), mm(info.sheet_mm.1)),
    };
    let name = n.name.clone().unwrap_or_default();
    let label_lines: Vec<String> =
        [name.trim().to_string(), size, material_name(frame.params.material).to_string()].into_iter().filter(|l| !l.is_empty()).collect();
    let input = LaserInput {
        cut: sheet.cut.clone(),
        sheet: sheet.sheet,
        mm_per_unit: frame.mm_per_unit,
        hoshi: info.hoshi.clone(),
        hoshi_d_mm: frame.params.hoshi_diameter_mm,
        label_lines,
        repeat_cell: info.cell,
    };
    let job = vectorcraft_stencil::laser_job(&input, lp);
    if job.sheets.is_empty() {
        let why = if job.warnings.is_empty() { "the laser job was refused".to_string() } else { job.warnings.join("; ") };
        return Err(disabled(cmd, why));
    }
    Ok(StencilSheets { sheets: job.sheets, warnings: job.warnings, base: file_safe(&name) })
}

/// `modifier.stencilExport`: a stencil object's drawn cut (the tile with tile output) as
/// kerf-compensated laser sheets, one SVG each.
pub(super) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let out = out_of(EXPORT, p)?;
    let id = stencil_object(EXPORT, s, p)?;
    let (_, kerf) = stencil_material(EXPORT, s, id)?;
    let lp = settings(s, kerf, p);
    if out == Out::Dialog {
        return Ok(json!({"dialog": "stencilExport", "id": id.0, "params": lp.to_json()}));
    }
    let job = stencil_sheets(EXPORT, s, id, &lp)?;
    let base = format!("{} laser", job.base);
    let files = emit(&job.sheets, &lp, &out, &base)?;
    let params = lp.to_json();
    // Remembered for the next export; a dry run (the dialog's summary) changes nothing.
    if out != Out::Summary {
        set_pref(s, Pref::Laser, params.clone());
    }
    Ok(json!({"id": id.0, "files": files, "sheets": job.sheets.len(), "warnings": job.warnings, "params": params}))
}

/// The material `p` names (`material`), else `stencil`'s, else paper.
pub(super) fn material_of(cmd: &str, p: &Value, stencil: Option<&Stencil>) -> Result<StencilMaterial> {
    match get(p, "material") {
        None => Ok(stencil.map_or(StencilMaterial::Paper, |st| st.material)),
        Some(v) => {
            let m = v.as_str().unwrap_or_default();
            StencilMaterial::ALL
                .into_iter()
                .find(|x| x.id().eq_ignore_ascii_case(m.trim()))
                .ok_or_else(|| bad(cmd, "`material` must be paper, film, card or custom"))
        }
    }
}

/// What the test card (or the power ladder) is made for: the selected stencil (when one is), the
/// material, the laser settings and the card's limits (`None` for the ladder of a custom material
/// without a stencil: the ladder doesn't use them).
pub(super) struct CardSetup {
    pub stencil: Option<NodeId>,
    pub material: StencilMaterial,
    pub lp: LaserParams,
    pub limits: Option<MaterialLimits>,
}

/// The card's settings: the material `p` names, else the selected (or named) stencil's, else
/// paper; the laser settings layered as for the export. `need_limits`: a custom material without
/// a selected stencil is refused (the card is drawn at the stencil's limits).
pub(super) fn card_setup(cmd: &str, s: &Session, p: &Value, need_limits: bool) -> Result<CardSetup> {
    let picked = match s.doc() {
        Ok(_) => super::super::targets(s, p).map_err(|e| bad(cmd, e.to_string()))?.first().copied(),
        Err(_) => None,
    };
    let stencil = match picked {
        Some(id) => {
            let doc = &s.doc()?.doc;
            let n = doc.node(id).ok_or_else(|| bad(cmd, format!("no object with id {}", id.0)))?;
            crate::picabia_stencil::frame(doc, n).map(|f| (id, f.params))
        }
        None => None,
    };
    let material = material_of(cmd, p, stencil.as_ref().map(|(_, st)| st))?;
    let lp = settings(s, material.limits().map(|l| l.kerf_mm), p);
    let limits = match (material.limits(), &stencil) {
        (Some(l), _) => Some(MaterialLimits { kerf_mm: lp.kerf_mm, ..l }),
        (None, Some((_, st))) => Some(MaterialLimits {
            min_cut_width_mm: st.min_cut_width_mm,
            min_bridge_mm: st.min_bridge_mm,
            tie_width_mm: st.tie_width_mm,
            min_corner_angle: st.min_corner_angle,
            kerf_mm: lp.kerf_mm,
        }),
        (None, None) if need_limits => {
            return Err(disabled(cmd, "a custom material's card needs a selected stencil object (its limits)"));
        }
        (None, None) => None,
    };
    Ok(CardSetup { stencil: stencil.map(|(id, _)| id), material, lp, limits })
}

/// The test card's sheet for `setup` (`None` without limits: see [`card_setup`]).
pub(super) fn card_sheet(setup: &CardSetup) -> Option<LaserSheet> {
    setup.limits.map(|l| vectorcraft_stencil::test_card(&l, material_name(setup.material), setup.lp.kerf_mm))
}

/// `kind`: the test card (false, the default) or the power ladder (true).
fn card_kind(cmd: &str, p: &Value) -> Result<bool> {
    match get(p, "kind") {
        None => Ok(false),
        Some(Value::String(k)) if k.trim().eq_ignore_ascii_case("card") => Ok(false),
        Some(Value::String(k)) if k.trim().eq_ignore_ascii_case("powerLadder") => Ok(true),
        Some(_) => Err(bad(cmd, "`kind` must be card or powerLadder")),
    }
}

/// `format`: G-code (true) or SVG (false). Default: SVG for the card, G-code for the ladder.
fn card_format(p: &Value, ladder: bool) -> Result<bool> {
    match get(p, "format") {
        None => Ok(ladder),
        Some(Value::String(f)) if f.trim().eq_ignore_ascii_case("svg") => Ok(false),
        Some(Value::String(f)) if f.trim().eq_ignore_ascii_case("gcode") => Ok(true),
        Some(_) => Err(bad(TEST_CARD, "`format` must be svg or gcode")),
    }
}

/// `modifier.stencilTestCard`: the card that measures a material's kerf and cutting limits, or
/// (`kind: powerLadder`, G-code only) the power ladder that finds its power and speed.
pub(super) fn test_card(s: &mut Session, p: &Value) -> Result<Value> {
    let out = out_of(TEST_CARD, p)?;
    let ladder = card_kind(TEST_CARD, p)?;
    let gcode = card_format(p, ladder)?;
    if ladder && !gcode {
        return Err(disabled(
            TEST_CARD,
            "the power ladder cuts each square at its own power and speed, which an SVG can't carry: export it as G-code (format: gcode)",
        ));
    }
    let setup = card_setup(TEST_CARD, s, p, !ladder)?;
    if out == Out::Dialog {
        let id = setup.stencil.map_or(Value::Null, |id| json!(id.0));
        let mut data = json!({"dialog": "stencilTestCard", "id": id, "material": setup.material.id(), "params": setup.lp.to_json()});
        data["format"] = json!(if gcode { "gcode" } else { "svg" });
        data["kind"] = json!(if ladder { "powerLadder" } else { "card" });
        if gcode {
            data["gcode"] = super::laser_gcode::job_params(s, setup.material, p).to_json();
        }
        return Ok(data);
    }
    if gcode {
        return super::laser_gcode::card_gcode(s, p, &out, &setup, ladder);
    }
    let card = card_sheet(&setup).ok_or_else(|| disabled(TEST_CARD, "the card needs the material's limits"))?;
    let base = format!("Test card {}", setup.material.id());
    let files = emit(std::slice::from_ref(&card), &setup.lp, &out, &base)?;
    let params = setup.lp.to_json();
    // Remembered for the next export; a dry run (the dialog's summary) changes nothing.
    if out != Out::Summary {
        set_pref(s, Pref::Laser, params.clone());
    }
    Ok(json!({"files": files, "sheets": 1, "warnings": [], "material": setup.material.id(), "params": params}))
}
