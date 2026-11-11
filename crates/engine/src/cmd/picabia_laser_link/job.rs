//! PICABIA: what `laser.start` and `laser.frame` run, with the box its moves reach (mm, work
//! coordinates): G-code text or a G-code file (untrusted: every line checked against a laser
//! allow-list, [`check_line`]), or a job from Picabia's own writers (the selected objects, a
//! stencil's sheet, the test card, the power ladder; checked the same way); and the machine profile
//! the link checks jobs against.

use kurbo::{Point, Rect};
use serde_json::{Value, json};
use vectorcraft_stencil::{Head, KnifeParams};

use super::{bad, get};
use crate::{EngineError, Result, Session};

/// The most G-code a job reads, text or file (bytes).
pub const MAX_GCODE_BYTES: u64 = 64 << 20;
/// The file extensions `path` takes.
const GCODE_EXTENSIONS: [&str; 6] = ["gcode", "nc", "ngc", "gc", "tap", "txt"];
/// The longest job name kept (characters).
const MAX_NAME_CHARS: usize = 80;

/// The machine the link checks jobs against: the saved machine profile (`laser.profile`, kept with
/// the preferences; the owner's X-Carve until changed), the fields the link needs.
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
    /// PICABIA (PIC35): what is in the tool clamp. Jobs are written for it and checked against its
    /// allow-list ([`check_line_for`]).
    pub tool: Head,
    /// The drag knife's settings: frames lift to its travel height first.
    pub knife: KnifeParams,
}

impl Machine {
    /// The session's machine profile.
    pub fn of(s: &Session) -> Machine {
        let p = crate::cmd::picabia_modifier::laser_gcode::MachineProfile::of(s);
        Machine {
            origin: if p.origin_at_head() { "head" } else { "machine" },
            name: p.name,
            bed_w_mm: p.bed_w_mm,
            bed_h_mm: p.bed_h_mm,
            s_max: p.s_max,
            rapid_mm_min: p.rapid_mm_min,
            air: p.air,
            checked: p.checked,
            tool: p.head,
            knife: p.knife,
        }
    }

    /// The knife is in the clamp.
    pub fn knife(&self) -> bool {
        self.tool == Head::Knife
    }

    pub fn origin_at_head(&self) -> bool {
        self.origin != "machine"
    }

    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "bedWidthMm": self.bed_w_mm, "bedHeightMm": self.bed_h_mm, "sMax": self.s_max, "rapidMmMin": self.rapid_mm_min, "air": self.air, "checked": self.checked, "origin": self.origin, "head": self.tool.id(), "knife": self.knife.to_json()})
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
/// A job for `laser.start`/`laser.frame`: its G-code, already cleaned into the lines the machine
/// gets, and where it goes.
#[derive(Clone, Debug)]
pub struct LaserRun {
    pub name: String,
    /// The lines to stream: comments and spaces dropped, each checked by [`check_line`] and by
    /// GRBL's own line rules ([`vectorcraft_grbl::clean_line`]).
    pub lines: Vec<String>,
    /// Work coordinates of every motion target, mm.
    pub bounds: Rect,
    pub estimated_sec: Option<f64>,
    pub warnings: Vec<String>,
    /// `gcode`, `file`, `design`, `stencil`, `testCard`, `powerLadder`.
    pub source: &'static str,
    /// "1 of 2" for a stencil's sheet.
    pub sheet: Option<String>,
    pub layer: Option<vectorcraft_doc::NodeId>,
}

/// The job `p` names: `gcode` (G-code text) or `path` (a G-code file), both untrusted and checked
/// line by line; else Picabia's own writers: `testCard` (card|powerLadder), or the objects
/// `ids`/`id` (default: the selection; a single stencil object's `sheet`).
pub fn build_run(cmd: &str, s: &Session, p: &Value) -> Result<LaserRun> {
    let given: Vec<&str> = ["gcode", "path", "testCard"].into_iter().filter(|k| get(p, k).is_some()).collect();
    if given.len() > 1 {
        return Err(bad(cmd, "give one job: `gcode`, `path` or `testCard`, not several"));
    }
    // The head is what is in the clamp, so the machine profile says, never one job (`head` isn't
    // a job param): a job written for the other head would cut with the wrong tool.
    let tool = Machine::of(s).tool;
    let untrusted = !given.is_empty() && given != ["testCard"];
    if untrusted {
        for k in ["ids", "id", "sheet"] {
            if get(p, k).is_some() {
                return Err(bad(cmd, format!("`{k}` goes with objects, not with `gcode` or `path`")));
            }
        }
    }
    let (name, text, source, writer) = match given.as_slice() {
        ["gcode"] => {
            let (name, text) = text_job(cmd, p)?;
            (name, text, "gcode", None)
        }
        ["path"] => {
            let (name, text) = file_job(cmd, p)?;
            (name, text, "file", None)
        }
        _ => {
            let t = crate::cmd::picabia_modifier::laser_run::trusted_run(cmd, s, p)?;
            let name = job_name(cmd, p, &t.name)?;
            (name, t.text.clone(), t.source, Some(t))
        }
    };
    let lines = check_gcode_for(&text, tool).map_err(|why| EngineError::Disabled(cmd.into(), why))?;
    // The box of the lines actually sent (cleaned as GRBL reads them: a `;` inside `( … )` doesn't
    // end the line), never of the raw text. The head starts at the work origin.
    let mut bounds = gcode_bounds(&lines.join("\n")).map_err(|why| EngineError::Disabled(cmd.into(), why))?.union_pt(Point::ZERO);
    let mut run = LaserRun { name, lines, bounds, estimated_sec: None, warnings: Vec::new(), source, sheet: None, layer: None };
    if let Some(t) = writer {
        // The writer's own box too (they agree; the union is never smaller than either).
        if [t.bounds.x0, t.bounds.y0, t.bounds.x1, t.bounds.y1].iter().all(|v| v.is_finite()) {
            bounds = bounds.union(t.bounds);
        }
        run.bounds = bounds;
        run.estimated_sec = Some(t.estimated_sec);
        run.warnings = t.warnings;
        run.sheet = t.sheet;
        run.layer = t.layer;
    }
    Ok(run)
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

/// G-code given as text: its name and text.
fn text_job(cmd: &str, p: &Value) -> Result<(String, String)> {
    let Some(Value::String(text)) = get(p, "gcode") else { return Err(bad(cmd, "`gcode` must be G-code text")) };
    if text.len() as u64 > MAX_GCODE_BYTES {
        return Err(bad(cmd, format!("`gcode` is longer than {} MB", MAX_GCODE_BYTES >> 20)));
    }
    if !text.is_ascii() {
        return Err(EngineError::Disabled(cmd.into(), "the G-code must be plain ASCII text".into()));
    }
    Ok((job_name(cmd, p, "G-code")?, text.clone()))
}

/// A G-code file (at most [`MAX_GCODE_BYTES`], ASCII text): its name and text.
fn file_job(cmd: &str, p: &Value) -> Result<(String, String)> {
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
    Ok((job_name(cmd, p, file)?, text))
}

/// The G-codes a laser job may use (×10: `G91.1` is 911): rapid and cut moves, arcs, dwell, the
/// default XY plane, millimetres, the default work coordinates and modes, absolute and relative
/// moves, feed per minute.
const ALLOWED_G: [i64; 16] = [0, 10, 20, 30, 40, 170, 210, 400, 490, 540, 610, 800, 900, 910, 911, 940];
/// The M-codes a laser job may use: end of program (`M2`, `M30`), the beam in dynamic laser mode
/// (`M4`) and off (`M5`), air assist (`M8`, `M9`).
const ALLOWED_M: [i64; 6] = [2, 4, 5, 8, 9, 30];

/// Every line of a laser job: comments and spaces dropped and checked by [`check_line`] and GRBL's
/// line rules. Returns the lines to send (empty ones left out), or why not, naming the line.
pub fn check_gcode(text: &str) -> std::result::Result<Vec<String>, String> {
    check_gcode_for(text, Head::Laser)
}

/// [`check_gcode`] for a job on `tool`: each line checked by [`check_line_for`].
pub fn check_gcode_for(text: &str, tool: Head) -> std::result::Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if out.len() >= vectorcraft_grbl::MAX_JOB_LINES {
            return Err(format!("the job has more than {} lines", vectorcraft_grbl::MAX_JOB_LINES));
        }
        let shown: String = raw.trim().chars().take(40).collect();
        let at = |why: String| format!("line {} (`{shown}`): {why}", i + 1);
        let clean = vectorcraft_grbl::clean_line(raw).map_err(|why| at(format!("the line {why}")))?;
        if clean.is_empty() {
            continue;
        }
        check_line_for(&clean, tool).map_err(at)?;
        out.push(clean);
    }
    if out.is_empty() {
        return Err("the G-code has no lines to run".into());
    }
    Ok(out)
}

/// One cleaned line against the laser allow-list (GRBL 1.1's documented G-code, narrowed to what a
/// laser job needs): no `$` system commands (settings, `$H`, `$J=` jogs, resets), no Z or other
/// axes, no tool changes, no plane, unit or coordinate-system changes, no `M3` (constant power),
/// and only the G- and M-codes in [`ALLOWED_G`] and [`ALLOWED_M`].
pub fn check_line(line: &str) -> std::result::Result<(), String> {
    check_line_for(line, Head::Laser)
}

/// One cleaned line against `tool`'s allow-list. The laser's is [`check_line`]'s. PICABIA (PIC35):
/// the knife's is the same but for two differences, both because the knife works on Z and has no
/// beam: Z moves are allowed (the blade goes down to cut and up to travel), and nothing that could
/// turn the spindle output on is (`M3`, `M4`, an `S` above 0), since the laser may still be wired
/// to it: a knife job that fires it would burn whatever is under the head. An `S` above 0 is also
/// the sign of a laser file opened by mistake.
pub fn check_line_for(line: &str, tool: Head) -> std::result::Result<(), String> {
    let knife = tool == Head::Knife;
    let upper = line.to_ascii_uppercase();
    if upper.starts_with('$') {
        return Err("`$` system commands (settings, homing, jogs, resets) aren't allowed in a job".into());
    }
    if upper.starts_with('%') {
        return Err("`%` program markers aren't allowed in a job".into());
    }
    let words = words(&upper)?;
    let has_g4 = words.iter().any(|(l, v)| *l == 'G' && (v * 10.0).round() as i64 == 40);
    for (letter, value) in &words {
        let code = (value * 10.0).round() as i64;
        match letter {
            'G' => match code {
                c if ALLOWED_G.contains(&c) => {}
                200 => return Err("G20 (inches) isn't allowed: export the job in millimetres (G21)".into()),
                180 | 190 => return Err(format!("G{} changes the arc plane; laser jobs stay in the XY plane (G17)", trim_num(*value))),
                280 | 281 | 300 | 301 | 920 | 921 | 922 | 923 | 100 | 530 => {
                    return Err(format!("G{} moves or offsets in a way that can't be checked against the bed", trim_num(*value)));
                }
                _ => return Err(format!("G{} isn't allowed in a laser job", trim_num(*value))),
            },
            'M' if knife && (code == 30 || code == 40) => {
                return Err(format!("M{} turns the spindle (the laser) on; a drag knife job never does", trim_num(*value)));
            }
            'M' => match code {
                c if c % 10 == 0 && ALLOWED_M.contains(&(c / 10)) => {}
                30 => return Err("M3 (constant power) isn't allowed: laser jobs use M4 (dynamic power, beam off while the head stops)".into()),
                _ => return Err(format!("M{} isn't allowed in a laser job (only M2, M4, M5, M8, M9, M30)", trim_num(*value))),
            },
            'X' | 'Y' | 'I' | 'J' | 'R' | 'N' => {}
            'F' if *value > 0.0 => {}
            'F' => return Err("the feed (F) must be more than 0".into()),
            'S' if knife && *value > 0.0 => {
                return Err("S sets the laser's power; a drag knife job has none (is this a laser file? the head is the knife)".into());
            }
            'S' if *value >= 0.0 => {}
            'S' => return Err("the power (S) can't be negative".into()),
            'P' if has_g4 && *value >= 0.0 => {}
            'P' => return Err("P is allowed only as a G4 dwell time".into()),
            'Z' if knife => {}
            'Z' => return Err("Z moves aren't allowed in a laser job".into()),
            c => return Err(format!("the word `{c}` isn't allowed in a laser job")),
        }
    }
    Ok(())
}

/// The work-coordinate box every move of `text` reaches (mm), arcs as their whole circle. A line's
/// modal words (`G20`/`G21`, `G90`/`G91`, the motion mode) apply before its coordinates, wherever
/// they stand on the line, as GRBL applies them. Refused (the reason) when the file moves in ways
/// that can't be checked against the bed: machine coordinates (`G53`), returns to stored positions
/// (`G28`, `G30`), coordinate offsets (`G92`, `G10`), other work coordinate systems (`G54`–`G59`
/// other than `G54`), relative moves (`G91`) from a position the file never set, or nothing that
/// moves.
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
        // The line's modal G-words first.
        for (letter, value) in &words {
            if *letter != 'G' {
                continue;
            }
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
                        trim_num(*value)
                    ));
                }
                550..=593 => {
                    return Err(format!(
                        "line {}: G{} picks another work coordinate system, which Picabia can't check against the bed",
                        i + 1,
                        trim_num(*value)
                    ));
                }
                _ => {}
            }
        }
        // Then its coordinates, in the units and mode the line set.
        let (mut x, mut y, mut ci, mut cj, mut r) = (None, None, None, None, None);
        for (letter, value) in &words {
            match letter {
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
            // The whole circle: an arc can bulge past its end points. With I/J the centre is the
            // start plus the offset (GRBL's incremental arc centres); with R the centre is within
            // R of the start, so every point is within 2R of it.
            let (cx, cy, rad) = match r {
                Some(r) => (start.0, start.1, r.abs() * 2.0),
                None => {
                    let (i, j) = (ci.unwrap_or(0.0), cj.unwrap_or(0.0));
                    (start.0 + i, start.1 + j, i.hypot(j))
                }
            };
            if rad.is_finite() && cx.is_finite() && cy.is_finite() {
                grow(&mut b, cx - rad, cy - rad);
                grow(&mut b, cx + rad, cy + rad);
            } else {
                return Err(format!("line {}: an arc out of range", i + 1));
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
