//! PICABIA: what `laser.start` and `laser.frame` run: G-code text, a G-code file, or the test card
//! or power ladder, with the box its moves reach (mm, work coordinates); and the machine profile
//! the link checks jobs against.

use kurbo::{Point, Rect};
use serde_json::{Value, json};
use vectorcraft_modifiers::StencilMaterial;
use vectorcraft_stencil::{GcodeParams, LadderParams};

use super::{bad, get};
use crate::{EngineError, Result, Session};

/// The most G-code a job reads, text or file (bytes).
pub const MAX_GCODE_BYTES: u64 = 64 << 20;
/// The file extensions `path` takes.
const GCODE_EXTENSIONS: [&str; 6] = ["gcode", "nc", "ngc", "gc", "tap", "txt"];
/// The longest job name kept (characters).
const MAX_NAME_CHARS: usize = 80;

/// The machine the link checks jobs against.
///
/// PICABIA: the saved machine profile (`laser.profile`, kept with the preferences) is ported
/// separately; until it lands every session uses these defaults (the owner's X-Carve, guesses
/// until checked on the machine). [`Machine::of`] is the one place that reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub name: String,
    /// The bed, mm (X across, Y front to back).
    pub bed_w_mm: f64,
    pub bed_h_mm: f64,
    /// `S` at full power (GRBL `$30`); the connected machine's own `$30` wins for jobs.
    pub s_max: f64,
    /// Travel (beam off) speed for framing and jogs, mm/min.
    pub rapid_mm_min: f64,
    /// Air assist (`M8`/`M9`).
    pub air: bool,
    /// The values were checked on the machine; false: guesses.
    pub checked: bool,
    /// Work 0,0 is where the person pressed Set origin (the sheet's bottom-left: `"head"`), or the
    /// bed's own (`"machine"`).
    pub origin: &'static str,
}

impl Default for Machine {
    fn default() -> Self {
        Machine {
            name: "X-Carve 23x48 + J Tech 2.8W".into(),
            bed_w_mm: 584.2,
            bed_h_mm: 1219.2,
            s_max: 1000.0,
            rapid_mm_min: 3000.0,
            air: false,
            checked: false,
            origin: "head",
        }
    }
}

impl Machine {
    /// The session's machine profile.
    pub fn of(_s: &Session) -> Machine {
        Machine::default()
    }

    pub fn origin_at_head(&self) -> bool {
        self.origin != "machine"
    }

    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "bedWidthMm": self.bed_w_mm, "bedHeightMm": self.bed_h_mm, "sMax": self.s_max, "rapidMmMin": self.rapid_mm_min, "air": self.air, "checked": self.checked, "origin": self.origin})
    }
}

/// The bed jobs must stay on: the profile's, or less where the connected machine's `$130`/`$131`
/// travel is smaller.
pub fn bed_mm(s: &Session) -> (f64, f64) {
    let prof = Machine::of(s);
    let (tx, ty) = s.picabia.laser.machine_travel();
    let pick = |bed: f64, travel: Option<f64>| match travel {
        Some(t) if t.is_finite() && t > 0.0 => bed.min(t),
        _ => bed,
    };
    (pick(prof.bed_w_mm, tx), pick(prof.bed_h_mm, ty))
}

/// `v`, or 0 when it isn't finite.
fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// `r` as `{x0, y0, x1, y1}` (finite).
pub fn rect_json(r: Rect) -> Value {
    json!({"x0": finite(r.x0), "y0": finite(r.y0), "x1": finite(r.x1), "y1": finite(r.y1)})
}

/// Seconds, rounded to tenths.
fn secs(v: f64) -> f64 {
    (finite(v) * 10.0).round() / 10.0
}

/// A job for `laser.start`/`laser.frame`: its G-code and where it goes.
#[derive(Clone, Debug)]
pub struct LaserRun {
    pub name: String,
    pub text: String,
    /// Work coordinates of every motion target, mm.
    pub bounds: Rect,
    pub estimated_sec: Option<f64>,
    pub warnings: Vec<String>,
    /// `gcode`, `file`, `testCard`, `powerLadder`.
    pub source: &'static str,
}

/// The job `p` names: `gcode` (G-code text), `path` (a G-code file) or `testCard`
/// (card|powerLadder), exactly one of them.
pub fn build_run(cmd: &str, s: &Session, p: &Value) -> Result<LaserRun> {
    let given: Vec<&str> = ["gcode", "path", "testCard"].into_iter().filter(|k| get(p, k).is_some()).collect();
    match given.as_slice() {
        ["gcode"] => text_run(cmd, p),
        ["path"] => file_run(cmd, p),
        ["testCard"] => card_run(cmd, s, p),
        [] => Err(bad(cmd, "give the job: `gcode` (G-code text), `path` (a G-code file) or `testCard` (card|powerLadder)")),
        _ => Err(bad(cmd, "give one job: `gcode`, `path` or `testCard`, not several")),
    }
}

/// A plain name from `name` (control characters dropped, cut to fit), else `default`.
fn job_name(cmd: &str, p: &Value, default: &str) -> Result<String> {
    match get(p, "name") {
        None => Ok(default.to_string()),
        Some(Value::String(n)) => {
            let n: String = n.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
            Ok(if n.trim().is_empty() { default.to_string() } else { n.trim().to_string() })
        }
        Some(_) => Err(bad(cmd, "`name` must be text")),
    }
}

/// G-code given as text.
fn text_run(cmd: &str, p: &Value) -> Result<LaserRun> {
    let Some(Value::String(text)) = get(p, "gcode") else { return Err(bad(cmd, "`gcode` must be G-code text")) };
    if text.len() as u64 > MAX_GCODE_BYTES {
        return Err(bad(cmd, format!("`gcode` is longer than {} MB", MAX_GCODE_BYTES >> 20)));
    }
    if !text.is_ascii() {
        return Err(EngineError::Disabled(cmd.into(), "the G-code must be plain ASCII text".into()));
    }
    let name = job_name(cmd, p, "G-code")?;
    gcode_run(cmd, name, text.clone(), "gcode")
}

/// A G-code file as a job: read (at most [`MAX_GCODE_BYTES`], ASCII text) and its bounds worked
/// out ([`gcode_bounds`]).
fn file_run(cmd: &str, p: &Value) -> Result<LaserRun> {
    let Some(Value::String(path)) = get(p, "path") else { return Err(bad(cmd, "`path` must be a G-code file path")) };
    let fp = std::path::Path::new(path);
    let ext = fp.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).unwrap_or_default();
    if path.trim().is_empty() || path.chars().any(char::is_control) || !GCODE_EXTENSIONS.contains(&ext.as_str()) {
        return Err(bad(cmd, "`path` must be a G-code file (.gcode, .nc, .ngc, .gc, .tap or .txt)"));
    }
    crate::file_access::check_read(path).map_err(|e| EngineError::Other(format!("{cmd}: {e}")))?;
    let bytes = crate::store::read_capped(fp, MAX_GCODE_BYTES).map_err(|e| EngineError::Other(format!("{cmd}: {e}")))?;
    let text = String::from_utf8(bytes).ok().filter(|t| t.is_ascii());
    let Some(text) = text else { return Err(EngineError::Disabled(cmd.into(), "the file isn't plain text G-code".into())) };
    let file = fp.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or("G-code");
    let name = job_name(cmd, p, file)?;
    gcode_run(cmd, name, text, "file")
}

fn gcode_run(cmd: &str, name: String, text: String, source: &'static str) -> Result<LaserRun> {
    // The head starts at the work origin.
    let bounds = gcode_bounds(&text).map_err(|why| EngineError::Disabled(cmd.into(), why))?.union_pt(Point::ZERO);
    Ok(LaserRun { name, text, bounds, estimated_sec: None, warnings: Vec::new(), source })
}

/// The test card or the power ladder, with the material's settings, the machine's `$30` and
/// `p`'s settings.
fn card_run(cmd: &str, s: &Session, p: &Value) -> Result<LaserRun> {
    let ladder = match get(p, "testCard") {
        Some(Value::String(k)) if k.trim().eq_ignore_ascii_case("card") => false,
        Some(Value::String(k)) if k.trim().eq_ignore_ascii_case("powerLadder") => true,
        _ => return Err(bad(cmd, "`testCard` must be card or powerLadder")),
    };
    let material = match get(p, "material") {
        None => StencilMaterial::Paper,
        Some(Value::String(m)) => StencilMaterial::ALL
            .into_iter()
            .find(|x| x.id().eq_ignore_ascii_case(m.trim()))
            .ok_or_else(|| bad(cmd, "`material` must be paper, film, card or custom"))?,
        Some(_) => return Err(bad(cmd, "`material` must be paper, film, card or custom")),
    };
    let prof = Machine::of(s);
    let machine_s_max = s.picabia.laser.machine_s_max();
    let mut gp = GcodeParams::for_material(material.id());
    gp.s_max = machine_s_max.unwrap_or(prof.s_max);
    gp.air = prof.air;
    gp.apply_json(p);
    gp.header =
        vec![format!("machine: {}", prof.name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c } else { '?' }).collect::<String>())];
    let mut warnings = Vec::new();
    if let Some(m) = machine_s_max
        && (gp.s_max - m).abs() > 1e-9
    {
        warnings.push(format!("sMax {} was replaced by the machine's $30 = {m}", gp.s_max));
        gp.s_max = m;
    }
    let (g, name) = if ladder {
        let l = vectorcraft_stencil::power_ladder(&LadderParams::from_json(p));
        (vectorcraft_stencil::ladder_gcode(&l, &gp), format!("Power ladder {}", material.id()))
    } else {
        let Some(mut limits) = material.limits() else {
            return Err(EngineError::Disabled(cmd.into(), "a custom material's test card needs a stencil's limits: pick paper, film or card".into()));
        };
        match get(p, "kerfMm") {
            None => {}
            Some(v) => match v.as_f64().filter(|k| k.is_finite() && (0.0..=2.0).contains(k)) {
                Some(k) => limits.kerf_mm = k,
                None => return Err(bad(cmd, "`kerfMm` must be a kerf in mm (0-2)")),
            },
        }
        let sheet = vectorcraft_stencil::test_card(&limits, material.id(), limits.kerf_mm);
        (vectorcraft_stencil::laser_gcode(&sheet, &gp), format!("Test card {}", material.id()))
    };
    if g.text.is_empty() {
        let why = if g.warnings.is_empty() { "the G-code was refused".to_string() } else { g.warnings.join("; ") };
        return Err(EngineError::Disabled(cmd.into(), format!("{name}: {why}")));
    }
    warnings.extend(g.warnings.iter().cloned());
    Ok(LaserRun {
        name,
        estimated_sec: Some(secs(vectorcraft_stencil::estimate_sec(&g, prof.rapid_mm_min))),
        bounds: g.bounds_mm,
        text: g.text,
        warnings,
        source: if ladder { "powerLadder" } else { "testCard" },
    })
}

/// The work-coordinate box every move of `text` reaches (mm), arcs as their whole circle. Refused
/// (the reason) when the file moves in ways that can't be checked against the bed: machine
/// coordinates (`G53`), returns to stored positions (`G28`, `G30`), coordinate offsets (`G92`,
/// `G10`), other work coordinate systems (`G54`–`G59` other than `G54`), relative moves (`G91`)
/// from a position the file never set, or nothing that moves.
pub fn gcode_bounds(text: &str) -> std::result::Result<Rect, String> {
    let mut pos = (0.0f64, 0.0f64);
    let mut abs = true;
    let mut scale = 1.0;
    let mut motion = 0u8;
    let mut b: Option<Rect> = None;
    let mut known = (false, false);
    let grow = |b: &mut Option<Rect>, x: f64, y: f64| {
        *b = Some(match *b {
            Some(r) => Rect::new(r.x0.min(x), r.y0.min(y), r.x1.max(x), r.y1.max(y)),
            None => Rect::new(x, y, x, y),
        });
    };
    for (i, raw) in text.lines().enumerate() {
        let line = strip_comments(raw).to_ascii_uppercase();
        let words = words(&line).map_err(|w| format!("line {}: {w}", i + 1))?;
        let (mut x, mut y, mut ci, mut cj, mut r) = (None, None, None, None, None);
        for (letter, value) in words {
            match letter {
                'G' => {
                    let g = (value * 10.0).round() as i64;
                    match g {
                        0 | 10 | 20 | 30 => motion = (g / 10) as u8,
                        900 => abs = true,
                        910 => abs = false,
                        200 => scale = 25.4,
                        210 => scale = 1.0,
                        530 | 280 | 281 | 300 | 301 | 920 | 921 | 922 | 923 | 100 => {
                            return Err(format!(
                                "line {}: G{} moves or offsets in a way Picabia can't check against the bed; export the G-code from Picabia instead",
                                i + 1,
                                trim_num(value)
                            ));
                        }
                        550..=593 => {
                            return Err(format!(
                                "line {}: G{} picks another work coordinate system, which Picabia can't check against the bed",
                                i + 1,
                                trim_num(value)
                            ));
                        }
                        _ => {}
                    }
                }
                'X' => x = Some(value * scale),
                'Y' => y = Some(value * scale),
                'I' => ci = Some(value * scale),
                'J' => cj = Some(value * scale),
                'R' => r = Some(value * scale),
                _ => {}
            }
        }
        if x.is_none() && y.is_none() {
            continue;
        }
        // A relative move from a position the file never set can't be checked.
        if !abs && ((x.is_some() && !known.0) || (y.is_some() && !known.1)) {
            return Err(format!(
                "line {}: a relative move (G91) from a position the file never set, which Picabia can't check against the bed",
                i + 1
            ));
        }
        if abs {
            known = (known.0 || x.is_some(), known.1 || y.is_some());
        }
        let start = pos;
        let to = |cur: f64, v: Option<f64>| match v {
            Some(v) if abs => v,
            Some(v) => cur + v,
            None => cur,
        };
        pos = (to(pos.0, x), to(pos.1, y));
        if !(pos.0.is_finite() && pos.1.is_finite()) || pos.0.abs() > 1e6 || pos.1.abs() > 1e6 {
            return Err(format!("line {}: a coordinate out of range", i + 1));
        }
        // Where the head starts isn't the file's: only where it moves to (arcs below).
        grow(&mut b, pos.0, pos.1);
        if motion == 2 || motion == 3 {
            // The whole circle: an arc can bulge past its end points.
            let rad = match (ci, cj, r) {
                (_, _, Some(r)) => r.abs(),
                (i, j, None) => i.unwrap_or(0.0).hypot(j.unwrap_or(0.0)),
            };
            let (cx, cy) = match r {
                Some(_) => (start.0, start.1),
                None => (start.0 + ci.unwrap_or(0.0), start.1 + cj.unwrap_or(0.0)),
            };
            let rad = if r.is_some() { rad * 2.0 } else { rad };
            if rad.is_finite() {
                grow(&mut b, cx - rad, cy - rad);
                grow(&mut b, cx + rad, cy + rad);
            }
        }
    }
    b.ok_or_else(|| "the G-code has no moves".to_string())
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `line` without `;…` and `(…)` comments.
fn strip_comments(line: &str) -> String {
    let line = line.split(';').next().unwrap_or("");
    let mut out = String::with_capacity(line.len());
    let mut depth = 0u32;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The `letter number` words of a comment-free, upper-case line (`$` lines and `%` have none).
fn words(line: &str) -> std::result::Result<Vec<(char, f64)>, String> {
    let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if line.starts_with('$') || line.starts_with('%') || line.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !c.is_ascii_alphabetic() {
            return Err(format!("`{c}` where a letter was expected"));
        }
        let start = at + c.len_utf8();
        let mut end = start;
        while let Some(&(j, d)) = chars.peek() {
            if d.is_ascii_digit() || matches!(d, '.' | '-' | '+') {
                end = j + d.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let v = line.get(start..end).and_then(|t| t.parse::<f64>().ok()).filter(|v| v.is_finite());
        let Some(v) = v else { return Err(format!("`{c}` without a number")) };
        out.push((c, v));
    }
    Ok(out)
}
