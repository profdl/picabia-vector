//! PICABIA: the Laser window's job: what is cut (the selection, the test card, the power ladder or
//! a G-code file), the sheet of a stencil with several, the material and the operation table, and
//! what the job comes to (size, lines, time, warnings) from a dry run of the export command,
//! asked again a moment after a change.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::ops::{self, Ops};
use crate::VectorcraftApp;
use crate::i18n;
use crate::picabia_ui::{button, num, readable, wrapped};
use crate::theme::Tokens;
use crate::widgets;

/// Seconds the summary waits after a change before it asks again.
const DEBOUNCE: f64 = 0.3;
/// Seconds after which an unchanged job is asked again (the design may have been edited).
const STALE: f64 = 3.0;

/// What the job cuts.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Source {
    /// The selected objects (a single stencil object: its sheets).
    #[default]
    Selection,
    /// The material's test card.
    TestCard,
    /// The power ladder.
    PowerLadder,
    /// A G-code file (its path; empty: none chosen yet).
    File(String),
}

/// One sheet of the job.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub w_mm: f64,
    pub h_mm: f64,
    pub lines: u64,
    pub est_sec: Option<f64>,
}

/// What the job comes to.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Summary {
    /// Not asked yet (or a change is waiting).
    #[default]
    Pending,
    /// Nothing to ask: nothing selected.
    NothingSelected,
    /// A G-code file: nothing to work out before it runs.
    File,
    Ready {
        sheets: Vec<Sheet>,
        warnings: Vec<String>,
    },
    /// The command refused (the reason).
    Failed(String),
}

/// The job's settings and summary (UI state).
#[derive(Clone, Debug, Default)]
pub struct JobState {
    pub source: Source,
    /// The material picked (`None`: the command's choice, a stencil's own, else paper).
    pub material: Option<String>,
    /// The material the last dry run used.
    pub material_used: Option<String>,
    pub ops: Ops,
    /// The person edited the table: it's sent (else the material's remembered settings apply).
    pub ops_set: bool,
    /// The stencil sheet to cut, 1-based.
    pub sheet: usize,
    /// The last dry run cut a stencil object (its sheets), not a design's outlines.
    pub stencil: bool,
    /// The power ladder's powers and speeds, as the window shows them (always sent for the ladder).
    pub grid: super::ladder::Grid,
    /// The power ladder's grid from the last dry run (`ladder`: `{powersPct, speedsMmMin, squareMm,
    /// gapMm, passes}`).
    pub ladder: Option<Value>,
    pub summary: Summary,
    /// Open… was pressed: the window asks for a G-code file once it has drawn.
    pub want_file: bool,
    /// PICABIA (PIC35): the drag knife's settings (`laser.status`'s `profile.knife`) when the
    /// profile's head is the knife, else `None`: the knife cuts with them, not the operation table.
    pub knife: Option<Value>,
    /// The dry run's params and the document state it ran on (its uid and contents).
    ran: Option<(Value, (u64, usize))>,
    ran_at: f64,
    changed_at: f64,
}

impl JobState {
    /// The material shown: the one picked, else the one the job used, else paper.
    pub fn material_shown(&self) -> &str {
        self.material.as_deref().or(self.material_used.as_deref()).unwrap_or("paper")
    }

    /// The job's params for `laser.start` and `laser.frame` (`sel` the selection), or `None` when
    /// there's nothing to cut.
    pub fn params(&self, sel: &[NodeId]) -> Option<Value> {
        let mut p = match &self.source {
            Source::File(path) if path.is_empty() => return None,
            Source::File(path) => return Some(json!({"path": path})),
            Source::Selection if sel.is_empty() => return None,
            Source::Selection => {
                let mut p = json!({"ids": sel.iter().map(|id| id.0).collect::<Vec<_>>()});
                if self.sheets() > 1 {
                    p["sheet"] = json!(self.sheet.max(1));
                }
                p
            }
            Source::TestCard => json!({"testCard": "card"}),
            Source::PowerLadder => json!({"testCard": "powerLadder"}),
        };
        self.put_settings(&mut p);
        Some(p)
    }

    /// The params Start runs: `params` with the material and the operation table as the window
    /// shows them, so the G-code built at confirm is the job the checklist shows (a G-code file runs
    /// as it is).
    pub fn start_params(&self, params: Value) -> Value {
        let mut p = params;
        if matches!(self.source, Source::File(_)) {
            return p;
        }
        p["material"] = json!(self.material_shown());
        self.ops.put(&mut p);
        p
    }

    /// The job as the checklist shows it: what, its size and lines, and each operation's power and
    /// speed.
    pub fn checklist_lines(&self, selection: &str) -> Vec<String> {
        let what = match &self.source {
            Source::Selection => selection.to_string(),
            Source::TestCard => tl!("Test card").to_string(),
            Source::PowerLadder => tl!("Power ladder").to_string(),
            Source::File(path) => std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        let mut out = vec![what];
        if let Some(s) = self.chosen_sheet() {
            out.push(size_text(s.w_mm, s.h_mm, s.lines));
        }
        if !matches!(self.source, Source::File(_)) {
            out.push(format!("{}: {}", tl!("Material"), super::ops::material_label(self.material_shown())));
            // The ladder cuts each square at its own power and speed: its grid, then the labels'
            // engraving, not the operation table's cut and score.
            let ladder = matches!(self.source, Source::PowerLadder);
            if ladder && let Some(grid) = self.ladder.as_ref().and_then(ladder_text) {
                out.push(grid);
            }
            if let Some(k) = &self.knife {
                out.push(knife_text(k));
                return out;
            }
            for (key, op) in super::ops::OP_KEYS.iter().zip(self.ops.0.iter()).filter(|(k, op)| op.enabled && (!ladder || **k == "engrave")) {
                out.push(format!("{}: {} %, {} mm/min, {}×", super::ops::op_label(key), num(op.power), num(op.speed), op.passes));
            }
        }
        out
    }

    /// What Start's job comes to now, to check again at confirm: the dry run of `start` (Start's
    /// params, [`JobState::start_params`]) and the chosen sheet's size and lines. `None` for a
    /// G-code file (it runs as it is) or before a dry run.
    pub fn expected(&self, sel: &[NodeId], start: &Value) -> Option<super::run::Expected> {
        let (cmd, mut p) = self.dry_run(sel)?;
        for k in ["material", "cut", "score", "engrave", "sheet"] {
            if let Some(v) = start.get(k) {
                p[k] = v.clone();
            }
        }
        let s = self.chosen_sheet()?;
        Some(super::run::Expected { cmd, params: p, sheet: self.sheet.max(1), w_mm: s.w_mm, h_mm: s.h_mm, lines: s.lines })
    }

    /// The dry run: its command and params, or `None` when there's nothing to ask.
    pub fn dry_run(&self, sel: &[NodeId]) -> Option<(&'static str, Value)> {
        let (cmd, mut p) = match &self.source {
            Source::File(_) => return None,
            Source::Selection if sel.is_empty() => return None,
            Source::Selection => ("laser.gcode", json!({"ids": sel.iter().map(|id| id.0).collect::<Vec<_>>()})),
            Source::TestCard => ("modifier.stencilTestCard", json!({"format": "gcode", "kind": "card"})),
            Source::PowerLadder => ("modifier.stencilTestCard", json!({"format": "gcode", "kind": "powerLadder"})),
        };
        self.put_settings(&mut p);
        p["dryRun"] = json!(true);
        Some((cmd, p))
    }

    fn put_settings(&self, p: &mut Value) {
        if let Some(m) = &self.material {
            p["material"] = json!(m);
        }
        if self.ops_set {
            self.ops.put(p);
        }
        if matches!(self.source, Source::PowerLadder) {
            self.grid.put(p);
        }
    }

    /// How many sheets the last dry run found.
    pub fn sheets(&self) -> usize {
        match &self.summary {
            Summary::Ready { sheets, .. } => sheets.len(),
            _ => 0,
        }
    }

    /// The sheet to cut (of the last dry run).
    pub fn chosen_sheet(&self) -> Option<&Sheet> {
        match &self.summary {
            Summary::Ready { sheets, .. } => sheets.get(self.sheet.max(1) - 1).or(sheets.first()),
            _ => None,
        }
    }

    /// Read a dry run's result.
    pub fn take_result(&mut self, r: Result<Value, String>) {
        self.summary = match r {
            Ok(v) => {
                if !self.ops_set
                    && let Some(g) = v.get("gcode")
                {
                    self.ops.read(g);
                }
                self.stencil = v.get("id").is_some_and(|id| !id.is_null());
                self.material_used = v.get("material").and_then(Value::as_str).map(str::to_string);
                self.ladder = v.get("ladder").filter(|l| l.is_object()).cloned();
                let files: Vec<&Value> = v.get("files").and_then(Value::as_array).into_iter().flatten().take(64).collect();
                let sheets: Vec<Sheet> = files
                    .iter()
                    .map(|f| Sheet {
                        name: f.get("sheet").or_else(|| f.get("name")).and_then(Value::as_str).unwrap_or_default().to_string(),
                        w_mm: f.get("wMm").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0),
                        h_mm: f.get("hMm").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0),
                        lines: f.get("lines").and_then(Value::as_u64).unwrap_or(0),
                        est_sec: f.get("estimatedSec").and_then(Value::as_f64).filter(|e| e.is_finite()),
                    })
                    .collect();
                let mut warnings = crate::picabia_ui::strings(&v, "warnings", 16);
                for file in &files {
                    for w in crate::picabia_ui::strings(file, "warnings", 8) {
                        if !warnings.contains(&w) {
                            warnings.push(w);
                        }
                    }
                }
                if self.sheet > sheets.len() || self.sheet == 0 {
                    self.sheet = 1;
                }
                Summary::Ready { sheets, warnings }
            }
            Err(e) => Summary::Failed(e),
        };
    }

    /// A setting changed at `now` (egui time): ask again a moment later.
    pub fn changed(&mut self, now: f64) {
        self.changed_at = now;
    }
}

/// The active document and its contents (every edit makes a new one): a dry run is asked again
/// when they change, not when the view or selection does.
fn doc_state(app: &VectorcraftApp) -> (u64, usize) {
    app.session.active().map_or((0, 0), |d| (d.uid, std::sync::Arc::as_ptr(&d.doc) as usize))
}

/// The selected objects, in selection order.
pub fn selection(app: &VectorcraftApp) -> Vec<NodeId> {
    app.session.active().map(|d| d.selection.objects.clone()).unwrap_or_default()
}

/// Ask the export command what the job comes to, when it changed a moment ago (or a while has
/// passed: the design may have been edited).
pub fn refresh(app: &mut VectorcraftApp, ctx: &egui::Context, j: &mut JobState) {
    let sel = selection(app);
    let Some((cmd, params)) = j.dry_run(&sel) else {
        j.summary = match j.source {
            Source::File(_) => Summary::File,
            _ => Summary::NothingSelected,
        };
        j.ran = None;
        return;
    };
    let now = ctx.input(|i| i.time);
    let state = doc_state(app);
    let same = j.ran.as_ref().is_some_and(|(p, d)| *p == params && *d == state);
    if same && now - j.ran_at < STALE {
        return;
    }
    let wait = j.changed_at + DEBOUNCE - now;
    if j.ran.is_some() && !same && wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        return;
    }
    // Read-only and not undoable: run directly so a refusal shows in the window, not in the
    // status bar.
    let r = app.session.execute(cmd, &params).map_err(|e| crate::i18n::msg(&e.to_string()));
    j.take_result(r);
    j.ran = Some((params, state));
    j.ran_at = now;
}

/// "Rectangle", "Rectangle and 2 more", or `None` when nothing is selected.
pub fn selection_name(app: &VectorcraftApp) -> Option<String> {
    let sel = selection(app);
    let doc = &app.session.active()?.doc;
    let first =
        sel.first().and_then(|id| doc.node(*id)).map(|n| n.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| tl!("Object").to_string()))?;
    Some(match sel.len() {
        1 => first,
        n => i18n::fmt(&i18n::tn(n as u64 - 1, "{name} and {n} more", "{name} and {n} more"), &[("name", &first)]),
    })
}

/// The source picker, the selection or file, the sheet, the material and the operation table;
/// whether a setting changed. `locked`: a job is running (nothing changes under it).
pub fn settings(app: &mut VectorcraftApp, ui: &mut Ui, j: &mut JobState, locked: bool) -> bool {
    let stencil = j.stencil && selection(app).len() == 1;
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let labels = ["Selection", "Test card", "Power ladder", "G-code file"];
    let active = match j.source {
        Source::Selection => 0,
        Source::TestCard => 1,
        Source::PowerLadder => 2,
        Source::File(_) => 3,
    };
    ui.add_enabled_ui(!locked, |ui| {
        if let Some(i) = widgets::tab_bar(ui, &labels, active)
            && i != active
        {
            j.source = match i {
                0 => Source::Selection,
                1 => Source::TestCard,
                2 => Source::PowerLadder,
                _ => Source::File(String::new()),
            };
            j.summary = Summary::Pending;
            j.ran = None;
            changed = true;
        }
    });
    ui.add_space(4.0);
    match &j.source {
        Source::Selection => match selection_name(app) {
            Some(name) if stencil => {
                wrapped(ui, &i18n::fmt(tl!("Cuts the stencil {name}: its sheet's cuts, scores and label."), &[("name", &name)]), t.text_dim)
            }
            Some(name) => wrapped(ui, &i18n::fmt(tl!("Cuts {name}: closed outlines are cut, open paths scored."), &[("name", &name)]), t.text_dim),
            None => wrapped(
                ui,
                tl!("Nothing selected: select the objects to cut, or choose the test card, the power ladder or a G-code file."),
                readable(t.warning, &t),
            ),
        },
        Source::TestCard => wrapped(ui, tl!("The material's test card: bridges, holes, slits and a kerf gauge."), t.text_dim),
        Source::PowerLadder => {
            wrapped(ui, tl!("Squares at each power and speed, to find the material's settings: powers across, speeds down."), t.text_dim)
        }
        Source::File(path) => {
            let named = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty());
            let mut open = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(named.as_deref().unwrap_or(tl!("No file chosen"))).size(12.0).color(if named.is_some() {
                    t.text
                } else {
                    t.text_dim
                }));
                open = button(ui, tl!("Open…"), false, !locked);
            });
            if open {
                j.want_file = true;
            }
        }
    }
    if matches!(j.source, Source::File(_)) {
        return changed;
    }
    if matches!(j.source, Source::PowerLadder) {
        ui.add_space(2.0);
        ui.add_enabled_ui(!locked, |ui| {
            if super::ladder::table(ui, &mut j.grid) {
                changed = true;
            }
        });
    }
    // A stencil with several sheets: which one.
    if j.sheets() > 1 {
        let labels: Vec<String> = match &j.summary {
            Summary::Ready { sheets, .. } => sheets.iter().map(sheet_label).collect(),
            _ => Vec::new(),
        };
        let shown = j.chosen_sheet().map(sheet_label).unwrap_or_default();
        ui.horizontal(|ui| {
            super::lead(ui, tl!("Sheet"));
            ui.add_enabled_ui(!locked, |ui| {
                let names: Vec<&str> = labels.iter().map(String::as_str).collect();
                if let Some(i) = widgets::dropdown_names_named(ui, "picabia_laser_sheet", Some(tl!("Sheet")), &shown, &names, 220.0) {
                    j.sheet = i + 1;
                    changed = true;
                }
            });
        });
    }
    ui.horizontal(|ui| {
        super::lead(ui, tl!("Material"));
        ui.add_enabled_ui(!locked, |ui| {
            if let Some(m) = ops::material_dropdown(ui, "panel", j.material_shown(), 140.0) {
                j.material = Some(m);
                // The table follows the material's remembered settings until edited again.
                j.ops_set = false;
                changed = true;
            }
        });
    });
    ui.add_space(2.0);
    if let Some(k) = &j.knife {
        // The knife cuts with the profile's settings; the table's power means nothing to it.
        wrapped(ui, &knife_text(k), t.text);
        wrapped(ui, tl!("The drag knife cuts with the machine profile's depths, feeds and blade offset (below). Engraving is left out."), t.text_dim);
        return changed;
    }
    ui.add_enabled_ui(!locked, |ui| {
        if ops::table(ui, "panel", &mut j.ops) {
            j.ops_set = true;
            changed = true;
        }
    });
    changed
}

/// "Knife: cut at Z −0.5 mm, 1500 mm/min, 1×; blade offset 0.25 mm" from the profile's knife.
pub fn knife_text(k: &Value) -> String {
    let n = |key: &str, d: f64| k.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
    i18n::fmt(
        tl!("Knife: cut at Z {z} mm, {feed} mm/min, {passes}×; blade offset {offset} mm"),
        &[
            ("z", &num(n("cutZMm", -0.5))),
            ("feed", &num(n("feedMmMin", 1500.0))),
            ("passes", &num(n("passes", 1.0))),
            ("offset", &num(n("offsetMm", 0.25))),
        ],
    )
}

/// "270.1 × 174.1 mm · 908 lines": a job's size and lines as the checklist shows them.
pub fn size_text(w: f64, h: f64, lines: u64) -> String {
    format!("{} · {}", i18n::fmt(tl!("{w} × {h} mm"), &[("w", &num(w)), ("h", &num(h))]), i18n::tn(lines, "{n} line", "{n} lines"))
}

/// The power ladder's grid in words: "Squares at 20–100 % power (5 steps) × 150–1200 mm/min
/// (4 steps), 1 pass each" (`v`: the dry run's `ladder`).
pub fn ladder_text(v: &Value) -> Option<String> {
    let list = |k: &str| -> Vec<f64> {
        v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_f64).filter(|x| x.is_finite()).collect()
    };
    let range = |xs: &[f64]| -> Option<String> {
        let lo = xs.iter().copied().reduce(f64::min)?;
        let hi = xs.iter().copied().reduce(f64::max)?;
        Some(if (hi - lo).abs() < 1e-9 { num(lo) } else { format!("{}–{}", num(lo), num(hi)) })
    };
    let (powers, speeds) = (list("powersPct"), list("speedsMmMin"));
    let passes = v.get("passes").and_then(Value::as_u64).unwrap_or(1);
    Some(i18n::fmt(
        tl!("Squares at {powers} % power ({p} steps) × {speeds} mm/min ({s} steps), {passes} each"),
        &[
            ("powers", &range(&powers)?),
            ("p", &powers.len().to_string()),
            ("speeds", &range(&speeds)?),
            ("s", &speeds.len().to_string()),
            ("passes", &i18n::tn(passes, "{n} pass", "{n} passes")),
        ],
    ))
}

/// "1 of 2 · 600 × 400 mm".
fn sheet_label(s: &Sheet) -> String {
    let size = i18n::fmt(tl!("{w} × {h} mm"), &[("w", &num(s.w_mm)), ("h", &num(s.h_mm))]);
    if s.name.is_empty() { size } else { format!("{} · {size}", s.name) }
}

/// What the job comes to: its size, lines and time, and what it warns of.
pub fn summary(ui: &mut Ui, j: &JobState) {
    let t = Tokens::get(ui.ctx());
    match &j.summary {
        Summary::Pending => crate::picabia_ui::note(ui, tl!("Working out the job…")),
        Summary::NothingSelected | Summary::File => {}
        Summary::Failed(e) => wrapped(ui, e, t.error),
        Summary::Ready { warnings, .. } => {
            if let Some(s) = j.chosen_sheet() {
                let mut line = i18n::fmt(tl!("{w} × {h} mm"), &[("w", &num(s.w_mm)), ("h", &num(s.h_mm))]);
                line.push_str(" · ");
                line.push_str(&i18n::tn(s.lines, "{n} line", "{n} lines"));
                if let Some(e) = s.est_sec {
                    line.push_str(" · ");
                    line.push_str(&i18n::fmt(tl!("about {time}"), &[("time", &super::status::clock(e))]));
                }
                ui.label(egui::RichText::new(line).font(crate::theme::semibold(12.0)).color(t.text));
            }
            if matches!(j.source, Source::PowerLadder)
                && let Some(grid) = j.ladder.as_ref().and_then(ladder_text)
            {
                crate::picabia_ui::note(ui, &grid);
            }
            for w in warnings {
                wrapped(ui, &crate::i18n::msg(w), readable(t.warning, &t));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_follow_the_source() {
        let mut j = JobState::default();
        let sel = [NodeId(7)];
        assert_eq!(j.params(&[]), None, "nothing selected");
        assert_eq!(j.params(&sel), Some(json!({"ids": [7]})));
        j.material = Some("film".into());
        j.ops_set = true;
        let p = j.params(&sel).unwrap();
        assert_eq!(p["material"], "film");
        assert!(p["cut"].is_object());
        assert!(p.get("powersPct").is_none(), "the grid is the ladder's only");
        j.source = Source::PowerLadder;
        j.grid.power = super::super::ladder::Range { from: 10.0, to: 30.0, steps: 3 };
        let p = j.params(&[]).unwrap();
        assert_eq!(p["testCard"], "powerLadder");
        assert_eq!(p["powersPct"], json!([10.0, 20.0, 30.0]), "the grid goes with the ladder");
        let (cmd, dry) = j.dry_run(&[]).unwrap();
        assert_eq!(cmd, "modifier.stencilTestCard");
        assert_eq!(dry["speedsMmMin"], json!([150.0, 300.0, 600.0, 1200.0]));
        j.source = Source::File(String::new());
        assert_eq!(j.params(&sel), None, "no file chosen");
        j.source = Source::File("/tmp/a.gcode".into());
        assert_eq!(j.params(&sel), Some(json!({"path": "/tmp/a.gcode"})));
        assert_eq!(j.dry_run(&sel), None);
    }

    #[test]
    fn a_dry_run_result_reads_leniently() {
        let mut j = JobState { sheet: 5, ..Default::default() };
        j.take_result(Ok(json!({
            "id": 3, "material": "card",
            "files": [{"sheet": "1 of 2", "wMm": 600, "hMm": 400, "lines": 120, "estimatedSec": 61.0, "warnings": ["w2"]}, {"name": 5}],
            "warnings": ["w1", 4],
            "gcode": {"cut": {"passes": 4}}
        })));
        assert!(j.stencil);
        assert_eq!(j.material_used.as_deref(), Some("card"));
        assert_eq!(j.sheet, 1, "out of range: back to the first");
        assert_eq!(j.sheets(), 2);
        assert_eq!(j.ops.0[0].passes, 4);
        let Summary::Ready { warnings, .. } = &j.summary else { panic!() };
        assert_eq!(warnings, &["w1".to_string(), "w2".to_string()]);
        j.take_result(Err("no".into()));
        assert_eq!(j.summary, Summary::Failed("no".into()));
    }
}
