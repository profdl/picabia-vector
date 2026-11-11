//! PICABIA: the jobs the laser link (`laser.start`, `laser.frame`) builds from Picabia's own G-code
//! writers: the objects `ids` (default: the selection) as `laser.gcode` cuts them, a single
//! stencil object's sheet as `modifier.stencilGcode` cuts it, or the test card / power ladder as
//! `modifier.stencilTestCard` cuts them, each with the material's saved operation settings and the
//! connected machine's `$30`. Nothing is written or remembered here.

use serde_json::Value;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::kurbo::Rect;
use vectorcraft_modifiers::StencilMaterial;

use super::laser::{bad, card_setup, get, stencil_material, stencil_sheets};
use super::laser_design::{design_sheet, single_stencil};
use super::laser_gcode::{MachineProfile, card_job, job_laser_params, job_params, material_param, sheet_gcode};
use crate::{EngineError, Result, Session};

/// A job written by Picabia.
#[derive(Clone, Debug)]
pub(crate) struct TrustedRun {
    pub name: String,
    pub text: String,
    /// The writer's own box of every motion target (work coordinates, mm).
    pub bounds: Rect,
    pub estimated_sec: f64,
    pub warnings: Vec<String>,
    /// `design`, `stencil`, `testCard`, `powerLadder`.
    pub source: &'static str,
    /// "1 of 2" for a stencil's sheet.
    pub sheet: Option<String>,
    pub layer: Option<NodeId>,
}

/// `p` with `sMax` dropped when the connected machine reports its own `$30` (which wins), and the
/// warning saying so.
fn machine_wins(s: &Session, p: &Value) -> (Value, Option<String>) {
    let machine = s.picabia.machine_reports().s_max.filter(|v| v.is_finite() && *v > 0.0);
    let mut p = p.clone();
    let mut warning = None;
    if let (Some(m), Some(o)) = (machine, p.as_object_mut())
        && let Some(given) = o.remove("sMax")
        && given.as_f64().is_none_or(|g| (g - m).abs() > 1e-9)
    {
        warning = Some(format!("sMax {given} was replaced by the machine's $30 = {m}"));
    }
    (p, warning)
}

/// Seconds, rounded to tenths (0 when not finite).
fn secs(v: f64) -> f64 {
    if v.is_finite() { (v * 10.0).round() / 10.0 } else { 0.0 }
}

/// The job `p` names: `testCard` (card|powerLadder), else the objects `ids`/`id` (default: the
/// selection), a single stencil object's sheet `sheet` (1-based; needed when it has several).
pub(crate) fn trusted_run(cmd: &str, s: &Session, p: &Value) -> Result<TrustedRun> {
    let (p, warning) = machine_wins(s, p);
    let p = &p;
    let prof = MachineProfile::of(s);
    let rapid = prof.rapid_mm_min;
    // What the job is called after: the tool that cuts it.
    let tool = if prof.head == vectorcraft_stencil::Head::Knife { "knife" } else { "laser" };
    let mut warnings: Vec<String> = warning.into_iter().collect();
    if let Some(v) = get(p, "testCard") {
        let ladder = match v {
            Value::String(k) if k.trim().eq_ignore_ascii_case("card") => false,
            Value::String(k) if k.trim().eq_ignore_ascii_case("powerLadder") => true,
            _ => return Err(bad(cmd, "`testCard` must be card or powerLadder")),
        };
        if get(p, "sheet").is_some() {
            return Err(bad(cmd, "`sheet` goes with a stencil object, not `testCard`"));
        }
        let setup = card_setup(cmd, s, p, !ladder)?;
        let (_, g, _, name) = card_job(cmd, s, &setup, p, ladder)?;
        warnings.extend(g.warnings.iter().cloned());
        return Ok(TrustedRun {
            name,
            estimated_sec: secs(vectorcraft_stencil::estimate_sec(&g, rapid)),
            bounds: g.bounds_mm,
            text: g.text,
            warnings,
            source: if ladder { "powerLadder" } else { "testCard" },
            sheet: None,
            layer: None,
        });
    }
    if s.doc().is_err() {
        return Err(EngineError::Disabled(
            cmd.into(),
            "no document open: give the job as `gcode`, `path` or `testCard`, or open a document and select what to cut".into(),
        ));
    }
    let Some(id) = single_stencil(s, p) else {
        // Any other objects: their outlines.
        for k in ["sheet", "kerfMm", "overlapMm", "label"] {
            if get(p, k).is_some() {
                return Err(bad(cmd, format!("`{k}` is for stencil objects only")));
            }
        }
        let material = material_param(cmd, p)?.unwrap_or(StencilMaterial::Paper);
        let gp = job_params(s, material, p);
        let d = design_sheet(cmd, s, p)?;
        let g = sheet_gcode(cmd, &d.sheet, &gp)?;
        warnings.extend(d.warnings);
        warnings.extend(g.warnings.iter().cloned());
        let layer = super::super::targets(s, p).ok().and_then(|ids| ids.first().copied());
        return Ok(TrustedRun {
            name: format!("{} {tool}", d.base),
            estimated_sec: secs(vectorcraft_stencil::estimate_sec(&g, rapid)),
            bounds: g.bounds_mm,
            text: g.text,
            warnings,
            source: "design",
            sheet: None,
            layer,
        });
    };
    let (params, kerf) = stencil_material(cmd, s, id)?;
    let material = material_param(cmd, p)?.unwrap_or(params.material);
    let lp = job_laser_params(s, kerf, p);
    let gp = job_params(s, material, p);
    let job = stencil_sheets(cmd, s, id, &lp)?;
    let n = job.sheets.len();
    let k = match get(p, "sheet") {
        None if n == 1 => 1,
        None => return Err(EngineError::Disabled(cmd.into(), format!("the stencil has {n} sheets: choose one with `sheet` (1-{n})"))),
        Some(v) => match v.as_f64() {
            Some(k) if k.is_finite() && k.fract() == 0.0 && k >= 1.0 && k <= n as f64 => k as usize,
            _ => return Err(bad(cmd, format!("`sheet` must be a sheet number, 1-{n}"))),
        },
    };
    let sheet = job.sheets.get(k - 1).ok_or_else(|| bad(cmd, format!("`sheet` must be a sheet number, 1-{n}")))?;
    let g = sheet_gcode(cmd, sheet, &gp)?;
    warnings.extend(job.warnings);
    warnings.extend(g.warnings.iter().cloned());
    Ok(TrustedRun {
        name: if n > 1 { format!("{} {tool} {k}", job.base) } else { format!("{} {tool}", job.base) },
        estimated_sec: secs(vectorcraft_stencil::estimate_sec(&g, rapid)),
        bounds: g.bounds_mm,
        text: g.text,
        warnings,
        source: "stencil",
        sheet: Some(format!("{k} of {n}")),
        layer: Some(id),
    })
}
