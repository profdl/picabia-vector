//! PICABIA: G-code for the laser: `modifier.stencilGcode` (a stencil object's sheets as GRBL 1.1
//! G-code), the test card and the power ladder as G-code (`modifier.stencilTestCard {format:
//! "gcode"}`) and the machine profile (`Prefs::picabia_machine`, `laser.profile`).
//!
//! Settings are layered: [`GcodeParams::for_material`], then the material's remembered operation
//! settings (`machine.materials`), then the machine's `S` max (the connected machine's `$30`, else
//! the profile's) and air assist, then the command's params. The laser settings (kerf, bed,
//! overlap, label) as for the SVG export, except that the bed is the machine's.
//!
//! **Text only**: G-code is written to files or returned; nothing here opens a port, connects,
//! arms or streams. What the connected machine reports (`$30`, `$130`/`$131`) reaches these
//! commands through [`super::LaserMachine`] in the session's Picabia state, which the laser link
//! fills; without one the profile's values are used.

use serde_json::{Map, Value, json};
use vectorcraft_geom::kurbo::Rect;
use vectorcraft_modifiers::StencilMaterial;
use vectorcraft_stencil::{Gcode, GcodeParams, Head, KnifeParams, LadderParams, LaserParams, LaserSheet};

use super::laser::{
    CardSetup, Out, Pref, bad, card_sheet, file_name, finite, get, out_of, pref, set_pref, sheet_path, stencil_material, stencil_object,
    stencil_sheets, write_out,
};
use crate::{EngineError, Result, Session};

pub(super) const GCODE: &str = "modifier.stencilGcode";
pub(super) const PROFILE: &str = "laser.profile";

/// The materials whose operation settings are remembered.
pub const MATERIALS: [&str; 4] = ["paper", "film", "card", "custom"];

/// The laser machine: its bed, power scale, travel speed, air assist and the operation settings
/// remembered per material. Defaults: the owner's X-Carve (23 × 48 in, a 2.8 W J Tech diode),
/// **guesses until checked on the machine** (`checked: false`).
#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile {
    pub name: String,
    /// The bed, mm (X across, Y front to back).
    pub bed_w_mm: f64,
    pub bed_h_mm: f64,
    /// `S` at full power (GRBL `$30`); the connected machine's own `$30` wins for jobs.
    pub s_max: f64,
    /// Travel (beam off) speed for estimates and framing, mm/min.
    pub rapid_mm_min: f64,
    /// Air assist (`M8`/`M9`).
    pub air: bool,
    /// The values were checked on the machine; false: guesses.
    pub checked: bool,
    /// Where a job's origin is: `"head"` (the default: the person jogs to the sheet's bottom-left
    /// and presses Set origin) or `"machine"` (work coordinates are the bed's).
    pub origin: String,
    /// Per material (`paper`, `film`, `card`, `custom`): `{cut, score, engrave}` op settings.
    pub materials: Map<String, Value>,
    /// PICABIA (PIC35): what is in the tool clamp: the laser (the default) or a drag knife on Z.
    /// Jobs on the machine are written and checked for this head.
    pub head: Head,
    /// The drag knife's settings (depths, feeds, blade offset), used when `head` is the knife.
    pub knife: KnifeParams,
}

impl Default for MachineProfile {
    fn default() -> Self {
        MachineProfile {
            name: "X-Carve 23x48 + J Tech 2.8W".into(),
            bed_w_mm: 584.2,
            bed_h_mm: 1219.2,
            s_max: 1000.0,
            rapid_mm_min: 3000.0,
            air: false,
            checked: false,
            origin: "head".into(),
            materials: Map::new(),
            head: Head::Laser,
            knife: KnifeParams::default(),
        }
    }
}

impl MachineProfile {
    pub const MIN_BED_MM: f64 = 10.0;
    pub const MAX_BED_MM: f64 = 5000.0;
    pub const MIN_RAPID: f64 = 10.0;
    pub const MAX_RAPID: f64 = 20_000.0;
    const MAX_NAME: usize = 80;

    /// The session's profile (`Prefs::picabia_machine`; none: the defaults).
    pub fn of(s: &Session) -> MachineProfile {
        Self::from_json(&pref(s, Pref::Machine))
    }

    /// A profile from JSON, leniently (see [`apply_json`](Self::apply_json)).
    pub fn from_json(v: &Value) -> MachineProfile {
        let mut p = MachineProfile::default();
        p.apply_json(v);
        p
    }

    /// Overwrite what `v` sets validly (`name`, `bedWidthMm`, `bedHeightMm`, `sMax`, `rapidMmMin`,
    /// `air`, `checked`, `origin`, `materials: {paper|film|card|custom: {cut?, score?, engrave?}}`,
    /// op objects overlaid field by field, `head` (`laser`|`knife`), `knife: {safeZMm?, cutZMm?,
    /// …}` overlaid field by field); wrong types are ignored, numbers clamped.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(n) = o.get("name").and_then(Value::as_str) {
            let n: String = n.chars().filter(|c| !c.is_control()).take(Self::MAX_NAME).collect();
            if !n.trim().is_empty() {
                self.name = n.trim().to_string();
            }
        }
        if let Some(x) = num("bedWidthMm") {
            self.bed_w_mm = x.clamp(Self::MIN_BED_MM, Self::MAX_BED_MM);
        }
        if let Some(x) = num("bedHeightMm") {
            self.bed_h_mm = x.clamp(Self::MIN_BED_MM, Self::MAX_BED_MM);
        }
        if let Some(x) = num("sMax") {
            self.s_max = x.clamp(1.0, GcodeParams::MAX_S_MAX);
        }
        if let Some(x) = num("rapidMmMin") {
            self.rapid_mm_min = x.clamp(Self::MIN_RAPID, Self::MAX_RAPID);
        }
        if let Some(b) = o.get("air").and_then(Value::as_bool) {
            self.air = b;
        }
        if let Some(b) = o.get("checked").and_then(Value::as_bool) {
            self.checked = b;
        }
        if let Some(v) = o.get("origin").and_then(Value::as_str).map(str::trim) {
            if v.eq_ignore_ascii_case("head") {
                self.origin = "head".into();
            } else if v.eq_ignore_ascii_case("machine") {
                self.origin = "machine".into();
            }
        }
        if let Some(h) = o.get("head").and_then(Value::as_str).and_then(Head::from_id) {
            self.head = h;
        }
        if let Some(k) = o.get("knife") {
            self.knife.apply_json(k);
        }
        if let Some(m) = o.get("materials").and_then(Value::as_object) {
            for id in MATERIALS {
                if let Some(ops) = m.get(id).filter(|v| v.is_object()) {
                    let mut gp = self.material_ops(id);
                    gp.apply_json(ops);
                    self.materials.insert(id.into(), ops_json(&gp));
                }
            }
        }
    }

    /// The profile as JSON (the [`apply_json`](Self::apply_json) names).
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "bedWidthMm": self.bed_w_mm,
            "bedHeightMm": self.bed_h_mm,
            "sMax": self.s_max,
            "rapidMmMin": self.rapid_mm_min,
            "air": self.air,
            "checked": self.checked,
            "origin": self.origin,
            "materials": Value::Object(self.materials.clone()),
            "head": self.head.id(),
            "knife": self.knife.to_json(),
        })
    }

    /// Material `id`'s starting settings with its remembered operations (the profile's `S` max
    /// and air not applied yet).
    fn material_ops(&self, id: &str) -> GcodeParams {
        let mut gp = GcodeParams::for_material(id);
        if let Some(ops) = self.materials.get(id) {
            gp.apply_json(ops);
        }
        gp
    }

    /// Jobs start at the head's work origin (`origin: "head"`).
    pub fn origin_at_head(&self) -> bool {
        self.origin != "machine"
    }

    /// Remember `gp`'s operations for material `id`.
    pub fn remember(&mut self, id: &str, gp: &GcodeParams) {
        if MATERIALS.contains(&id) {
            self.materials.insert(id.into(), ops_json(gp));
        }
    }
}

/// `{cut, score, engrave}` of `gp`.
fn ops_json(gp: &GcodeParams) -> Value {
    let all = gp.to_json();
    json!({"cut": all["cut"], "score": all["score"], "engrave": all["engrave"]})
}

/// The `material` param (paper|film|card|custom), `None` when not given.
pub(super) fn material_param(cmd: &str, p: &Value) -> Result<Option<StencilMaterial>> {
    match get(p, "material") {
        None => Ok(None),
        Some(v) => {
            let m = v.as_str().unwrap_or_default();
            StencilMaterial::ALL
                .into_iter()
                .find(|x| x.id().eq_ignore_ascii_case(m.trim()))
                .map(Some)
                .ok_or_else(|| bad(cmd, "`material` must be paper, film, card or custom"))
        }
    }
}

/// The bed jobs must stay on: the profile's, or less where the connected machine's `$130`/`$131`
/// travel is smaller.
pub(super) fn bed_mm(s: &Session) -> (f64, f64) {
    let prof = MachineProfile::of(s);
    let m = s.picabia.machine_reports();
    let pick = |bed: f64, travel: Option<f64>| match travel {
        Some(t) if t.is_finite() && t > 0.0 => bed.min(t),
        _ => bed,
    };
    (pick(prof.bed_w_mm, m.travel_x_mm), pick(prof.bed_h_mm, m.travel_y_mm))
}

/// The connected machine's `$30` (`None`: no machine connected, or it didn't say).
fn machine_s_max(s: &Session) -> Option<f64> {
    s.picabia.machine_reports().s_max.filter(|v| v.is_finite() && *v > 0.0)
}

/// The G-code settings for `material`: its starting values, the remembered operations, the
/// machine's `S` max (the connected machine's `$30`, else the profile's) and air, then `p`.
pub(super) fn job_params(s: &Session, material: StencilMaterial, p: &Value) -> GcodeParams {
    let prof = MachineProfile::of(s);
    let mut gp = prof.material_ops(material.id());
    gp.s_max = machine_s_max(s).unwrap_or(prof.s_max);
    gp.air = prof.air;
    gp.head = prof.head;
    gp.knife = prof.knife;
    gp.apply_json(p);
    gp.header = vec![format!("machine: {}", ascii(&prof.name)), format!("material: {}", material.id())];
    gp
}

/// The laser settings for G-code: as the SVG export's, with the bed the machine's. With the
/// knife as the profile's head, the kerf is 0 unless `p` gives one: a blade takes no width out,
/// so the material's beam kerf would grow every cut-out.
pub(super) fn job_laser_params(s: &Session, kerf: Option<f64>, p: &Value) -> LaserParams {
    let (w, h) = bed_mm(s);
    let mut lp = LaserParams::default();
    lp.apply_json(&pref(s, Pref::Laser));
    lp.apply_json(&json!({"bedWidthMm": w, "bedHeightMm": h}));
    if let Some(k) = kerf {
        lp.apply_json(&json!({"kerfMm": k}));
    }
    if MachineProfile::of(s).head == Head::Knife {
        lp.apply_json(&json!({"kerfMm": 0.0}));
    }
    lp.apply_json(p);
    lp
}

fn ascii(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c } else { '?' }).collect()
}

/// `r` as `{x0, y0, x1, y1}` (finite).
fn rect_json(r: Rect) -> Value {
    json!({"x0": finite(r.x0), "y0": finite(r.y0), "x1": finite(r.x1), "y1": finite(r.y1)})
}

/// Seconds, rounded to tenths.
fn secs(v: f64) -> f64 {
    (finite(v) * 10.0).round() / 10.0
}

/// One sheet's G-code, refused (empty text) as an error.
pub(super) fn sheet_gcode(cmd: &str, sheet: &LaserSheet, gp: &GcodeParams) -> Result<Gcode> {
    let g = match gp.head {
        Head::Laser => vectorcraft_stencil::laser_gcode(sheet, gp),
        Head::Knife => vectorcraft_stencil::knife_gcode(sheet, gp),
    };
    if g.text.is_empty() {
        let why = if g.warnings.is_empty() { "the G-code was refused".to_string() } else { g.warnings.join("; ") };
        return Err(EngineError::Disabled(cmd.into(), format!("sheet {}: {why}", sheet.name)));
    }
    Ok(g)
}

/// The lines of G-code `text` a job sends: what is left of each line once GRBL's comments and
/// spaces are taken out (the header's `;` comment lines aren't sent), as the laser link counts them.
pub(crate) fn sent_lines(text: &str) -> usize {
    text.lines().filter(|l| vectorcraft_grbl::clean_line(l).is_ok_and(|c| !c.is_empty())).count()
}

/// Write (or encode) each file and describe it.
fn emit(sheets: &[(&LaserSheet, Gcode)], out: &Out, base: &str, rapid: f64) -> Result<Vec<Value>> {
    let n = sheets.len();
    let mut files = Vec::with_capacity(n);
    for (i, (sheet, g)) in sheets.iter().enumerate() {
        let mut f = json!({
            "sheet": sheet.name,
            "wMm": finite(sheet.w_mm),
            "hMm": finite(sheet.h_mm),
            "lines": sent_lines(&g.text),
            "estimatedSec": secs(vectorcraft_stencil::estimate_sec(g, rapid)),
            "burnMm": finite(g.burn_mm),
            "travelMm": finite(g.travel_mm),
            "boundsMm": rect_json(g.bounds_mm),
            "warnings": g.warnings,
        });
        let name = sheet_path(&format!("{base}.gcode"), i + 1, n);
        match out {
            Out::Path(path) => {
                let path = sheet_path(path, i + 1, n);
                write_out(&path, g.text.as_bytes())?;
                f["name"] = json!(file_name(&path));
                f["path"] = json!(path);
            }
            Out::Data => {
                f["name"] = json!(name);
                f["data"] = json!(vectorcraft_format::base64_encode(g.text.as_bytes()));
            }
            Out::Summary | Out::Dialog => f["name"] = json!(name),
        }
        files.push(f);
    }
    Ok(files)
}

/// One sheet's G-code written (or encoded) and described, as [`emit`].
pub(super) fn emit_one(cmd: &str, sheet: &LaserSheet, gp: &GcodeParams, out: &Out, base: &str, rapid: f64) -> Result<Vec<Value>> {
    let g = sheet_gcode(cmd, sheet, gp)?;
    emit(&[(sheet, g)], out, base, rapid)
}

/// Remember `gp`'s operations for `material` in the machine profile.
pub(super) fn remember_ops(s: &mut Session, material: StencilMaterial, gp: &GcodeParams) {
    let mut prof = MachineProfile::of(s);
    prof.remember(material.id(), gp);
    set_pref(s, Pref::Machine, prof.to_json());
}

/// `modifier.stencilGcode`: a stencil object's sheets as GRBL G-code, one file per sheet.
pub(super) fn stencil_gcode(s: &mut Session, p: &Value) -> Result<Value> {
    let out = out_of(GCODE, p)?;
    let id = stencil_object(GCODE, s, p)?;
    let (params, kerf) = stencil_material(GCODE, s, id)?;
    let material = material_param(GCODE, p)?.unwrap_or(params.material);
    let lp = job_laser_params(s, kerf, p);
    let gp = job_params(s, material, p);
    if out == Out::Dialog {
        return Ok(
            json!({"dialog": "stencilExport", "id": id.0, "format": "gcode", "material": material.id(), "params": lp.to_json(), "gcode": gp.to_json()}),
        );
    }
    let job = stencil_sheets(GCODE, s, id, &lp)?;
    let mut parts = Vec::with_capacity(job.sheets.len());
    for sheet in &job.sheets {
        parts.push((sheet, sheet_gcode(GCODE, sheet, &gp)?));
    }
    let rapid = MachineProfile::of(s).rapid_mm_min;
    let files = emit(&parts, &out, &format!("{} laser", job.base), rapid)?;
    // Remembered per material for the next export; a dry run changes nothing.
    if out != Out::Summary {
        remember_ops(s, material, &gp);
    }
    Ok(json!({
        "id": id.0,
        "files": files,
        "sheets": job.sheets.len(),
        "warnings": job.warnings,
        "material": material.id(),
        "params": lp.to_json(),
        "gcode": gp.to_json(),
    }))
}

/// The test card's or the ladder's G-code and a name for it.
pub(super) fn card_job(cmd: &str, s: &Session, setup: &CardSetup, p: &Value, ladder: bool) -> Result<(LaserSheet, Gcode, GcodeParams, String)> {
    let gp = job_params(s, setup.material, p);
    if ladder {
        let l = vectorcraft_stencil::power_ladder(&LadderParams::from_json(p));
        let g = vectorcraft_stencil::ladder_gcode(&l, &gp);
        if g.text.is_empty() {
            return Err(EngineError::Disabled(cmd.into(), format!("the power ladder's G-code was refused: {}", g.warnings.join("; "))));
        }
        return Ok((l.sheet, g, gp, format!("Power ladder {}", setup.material.id())));
    }
    let sheet = card_sheet(setup).ok_or_else(|| EngineError::Disabled(cmd.into(), "the card needs the material's limits".into()))?;
    let g = sheet_gcode(cmd, &sheet, &gp)?;
    Ok((sheet, g, gp, format!("Test card {}", setup.material.id())))
}

/// `modifier.stencilTestCard {format: "gcode"}`: the test card or (`ladder`) the power ladder as
/// G-code.
pub(super) fn card_gcode(s: &mut Session, p: &Value, out: &Out, setup: &CardSetup, ladder: bool) -> Result<Value> {
    let cmd = super::laser::TEST_CARD;
    let (sheet, g, gp, base) = card_job(cmd, s, setup, p, ladder)?;
    let rapid = MachineProfile::of(s).rapid_mm_min;
    let warnings = g.warnings.clone();
    let files = emit(&[(&sheet, g)], out, &base, rapid)?;
    // The card's settings are remembered as the export's are; the ladder's per-square settings
    // aren't settings to remember.
    if *out != Out::Summary && !ladder {
        set_pref(s, Pref::Laser, setup.lp.to_json());
        remember_ops(s, setup.material, &gp);
    }
    let mut r = json!({
        "files": files,
        "sheets": 1,
        "warnings": warnings,
        "material": setup.material.id(),
        "kind": if ladder { "powerLadder" } else { "card" },
        "params": setup.lp.to_json(),
        "gcode": gp.to_json(),
    });
    if ladder {
        r["ladder"] = LadderParams::from_json(p).to_json();
    }
    Ok(r)
}

/// `laser.profile`: read the machine profile, or merge `set` into it.
pub(super) fn profile(s: &mut Session, p: &Value) -> Result<Value> {
    let mut prof = MachineProfile::of(s);
    match get(p, "set") {
        None => {}
        Some(v @ Value::Object(_)) => {
            prof.apply_json(v);
            set_pref(s, Pref::Machine, prof.to_json());
        }
        Some(_) => return Err(bad(PROFILE, "`set` must be an object of profile fields")),
    }
    let (w, h) = bed_mm(s);
    let machine = machine_s_max(s);
    Ok(json!({
        "profile": prof.to_json(),
        "bedMm": {"w": w, "h": h},
        "sMaxUsed": machine.unwrap_or(prof.s_max),
        "sMaxMachine": machine,
        "materialDefaults": MATERIALS.iter().map(|m| (m.to_string(), ops_json(&prof.material_ops(m)))).collect::<Map<String, Value>>(),
    }))
}
