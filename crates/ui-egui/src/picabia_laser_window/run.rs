//! PICABIA: running a laser job from the Laser window: the safety checklist that Start opens (and
//! the short confirm a paused job's Resume opens), the one place the app arms the laser, the job's
//! progress, and the emergency stop (the big Stop button, the bar that stays on screen while the
//! window is closed, and Shift+Esc anywhere in the app).
//!
//! # Arming
//! [`vectorcraft_engine::Session::laser_execute_armed`] is called from [`confirm`] only, after every
//! box of the checklist is ticked (a test checks that no other file of the app calls it). The
//! checklist is UI state, not a dialog: `ui.dialog.confirm`, commands, MCP and scripts can't reach
//! it. Input from automation arms only the simulator: clicks and keys the control channel injects
//! (`ui.click`, `ui.key`: a frame of synthetic input), a control request being answered, and
//! accessibility clients' actions (AccessKit `Click`: any screen-reader or UI-automation client can
//! send one). On a real machine the press must be a person's pointer press (where it came from is
//! recorded when the button goes down, not when it is released) and every box a person's tick;
//! boxes automation ticked are cleared. The motion commands (home, jog, unlock, set origin, frame)
//! go through [`as_person`] the same way.
//!
//! An open native menu (macOS menu tracking) runs no frames: the window's Stop and Shift+Esc wait
//! until it closes. The machine's own emergency stop is the one to reach for meanwhile (the
//! checklist asks that one is in reach).
//!
//! # Stop
//! Stop raises the link's stop flag through [`vectorcraft_engine::Session::laser_stop_handle`]
//! first (the worker acts on it within one serial read, whatever the UI thread does next), then
//! runs `laser.stop`, which confirms it.

use egui::{Align2, Color32, Context, CornerRadius, Id, Key, Modifiers, Sense, Ui, vec2};
use serde_json::{Value, json};

use super::Action;
use super::status::{self, Status};
use crate::VectorcraftApp;
use crate::picabia_ui::{button, readable, wrapped};
use crate::theme::{Tokens, semibold};
use crate::widgets;

/// What the checklist confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Start the job.
    Start,
    /// Resume a paused job (the beam comes back on).
    Resume,
}

impl Confirm {
    /// The boxes to tick, in order: the laser's, or (`knife`, PIC35) the drag knife's, which has no
    /// beam to guard against but a blade, a Z that must be zeroed on the sheet and maybe a laser
    /// still wired to the spindle output.
    pub fn checks(self, knife: bool) -> Vec<&'static str> {
        match self {
            Confirm::Start if knife => vec![
                tl!("The drag knife is in the clamp and the laser is unplugged"),
                tl!("Z was zeroed with the blade just touching the sheet"),
                tl!("The material lies flat and is held down"),
                tl!("Hands stay clear of the head while it moves"),
                tl!("Someone stays at the machine until the job ends"),
                tl!("A hardware emergency stop is in reach"),
            ],
            Confirm::Resume if knife => vec![tl!("Hands are clear of the head"), tl!("Nothing has moved and someone is at the machine")],
            Confirm::Start => vec![
                tl!("The laser shielding is in place"),
                tl!("Everyone near the machine wears laser safety glasses"),
                tl!("The laser is focused for this material"),
                tl!("Extraction or ventilation is on"),
                tl!("The material lies flat and is held down"),
                tl!("Someone stays at the machine until the job ends"),
                tl!("A hardware emergency stop is in reach"),
            ],
            Confirm::Resume => {
                vec![tl!("The shielding is in place and safety glasses are on"), tl!("Nothing has moved and someone is at the machine")]
            }
        }
    }

    fn title(self, knife: bool) -> &'static str {
        match self {
            Confirm::Start if knife => tl!("Before the knife starts"),
            Confirm::Start => tl!("Before the laser starts"),
            Confirm::Resume => tl!("Before the job resumes"),
        }
    }

    fn button(self) -> &'static str {
        match self {
            Confirm::Start => tl!("Start"),
            Confirm::Resume => tl!("Resume"),
        }
    }
}

/// The open checklist: what it confirms, the job's params (for Start) and what it is shown as, the
/// boxes ticked and whether automation ticked each, and where the confirm button's press came from.
#[derive(Clone, Debug)]
pub struct Checklist {
    pub kind: Confirm,
    pub params: Value,
    /// The job as the checklist shows it (name, size, power and speed): what Start runs.
    pub summary: Vec<String>,
    pub ticks: Vec<bool>,
    /// Box `i` was last ticked by automated input ([`automated_frame`]).
    pub ticked_by_automation: Vec<bool>,
    /// The confirm button's pointer press: `Some(automated)` from the frame it went down in; `None`
    /// before one (an AccessKit or keyboard press has none and counts as automated).
    pub press: Option<bool>,
    /// What the job came to when the checklist opened (Start): checked again at confirm, so the job
    /// that runs is the one the checklist showed.
    pub expected: Option<Expected>,
    /// PICABIA (PIC35): the drag knife's boxes, not the laser's.
    pub knife: bool,
}

/// A job's size and line count, and the dry run that tells them.
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub cmd: &'static str,
    pub params: Value,
    /// The sheet that runs, 1-based.
    pub sheet: usize,
    pub w_mm: f64,
    pub h_mm: f64,
    pub lines: u64,
}

impl Expected {
    /// The sheet's size and lines in a dry run's reply, if it has that sheet.
    pub fn read(&self, reply: &Value) -> Option<(f64, f64, u64)> {
        let f = reply.get("files")?.as_array()?.get(self.sheet.max(1) - 1)?;
        Some((f.get("wMm")?.as_f64()?, f.get("hMm")?.as_f64()?, f.get("lines")?.as_u64()?))
    }

    /// The job is still the one shown: the same size (to 0.01 mm) and line count.
    pub fn matches(&self, (w, h, lines): (f64, f64, u64)) -> bool {
        (w - self.w_mm).abs() <= 0.01 && (h - self.h_mm).abs() <= 0.01 && lines == self.lines
    }
}

impl Checklist {
    pub fn new(kind: Confirm, params: Value) -> Self {
        Self::for_head(kind, params, false)
    }

    /// The checklist for the laser, or (`knife`) for the drag knife.
    pub fn for_head(kind: Confirm, params: Value, knife: bool) -> Self {
        let n = kind.checks(knife).len();
        Checklist {
            kind,
            params,
            summary: Vec::new(),
            ticks: vec![false; n],
            ticked_by_automation: vec![false; n],
            press: None,
            expected: None,
            knife,
        }
    }

    /// Every box is ticked.
    pub fn complete(&self) -> bool {
        !self.ticks.is_empty() && self.ticks.iter().all(|t| *t)
    }

    /// Box `i` was clicked, in a frame of automated input or not.
    pub fn toggle(&mut self, i: usize, automated: bool) {
        if let (Some(t), Some(a)) = (self.ticks.get_mut(i), self.ticked_by_automation.get_mut(i)) {
            *t = !*t;
            *a = *t && automated;
        }
    }

    /// Untick what automated input ticked (a real machine needs a person's ticks).
    pub fn clear_automated_ticks(&mut self) {
        for (t, a) in self.ticks.iter_mut().zip(self.ticked_by_automation.iter_mut()) {
            if *a {
                *t = false;
                *a = false;
            }
        }
    }

    /// Some box was ticked by automated input.
    pub fn automated_ticks(&self) -> bool {
        self.ticks.iter().zip(&self.ticked_by_automation).any(|(t, a)| *t && *a)
    }
}

/// Why [`confirm`] didn't run the job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not every box is ticked.
    Unticked,
    /// The press or a tick came from automation (the control channel, a control request, an
    /// accessibility client) and the machine is a real one.
    NotAPerson,
    /// The link closed meanwhile.
    NotConnected,
    /// The job changed since the checklist opened (the design was edited): the checklist shows it
    /// again, to be confirmed afresh.
    Changed,
}

/// Whether a press counts as automated: where it came from when it went down (`press`; none: an
/// AccessKit or keyboard press, automated) or the frame it lands in being automated.
pub fn press_automated(press: Option<bool>, auto_now: bool) -> bool {
    press.unwrap_or(true) || auto_now
}

/// The job as it would be built now still matches what the checklist showed; when it doesn't, the
/// checklist takes the new numbers, its boxes are cleared and the reason is the result.
fn still_the_job(app: &mut VectorcraftApp, list: &mut Checklist) -> Result<(), String> {
    let Some(exp) = list.expected.clone() else { return Ok(()) };
    let reply = app.session.execute(exp.cmd, &exp.params).map_err(|e| e.to_string())?;
    let now = exp.read(&reply).ok_or_else(|| tl!("The job couldn't be worked out again: check it and press Start again.").to_string())?;
    if exp.matches(now) {
        return Ok(());
    }
    let old = super::job::size_text(exp.w_mm, exp.h_mm, exp.lines);
    let new = super::job::size_text(now.0, now.1, now.2);
    for line in list.summary.iter_mut().filter(|l| **l == old) {
        line.clone_from(&new);
    }
    list.expected = Some(Expected { w_mm: now.0, h_mm: now.1, lines: now.2, ..exp });
    list.ticks.iter_mut().for_each(|t| *t = false);
    list.ticked_by_automation.iter_mut().for_each(|t| *t = false);
    list.press = None;
    Err(tl!("The job changed since the checklist opened: check it and press Start again.").to_string())
}

/// Whether a confirmed checklist may arm the laser: always for a person's click, and for automated
/// input (see [`automated_frame`]) only on the simulator.
pub fn may_arm(automated: bool, sim: bool) -> bool {
    !automated || sim
}

/// Whether the input this frame came from automation: a frame carrying input the control channel
/// injected, a control request being answered, or an accessibility client's action (AccessKit's
/// `Click` and the rest: any screen-reader or UI-automation client can send them).
pub fn automated_frame(app: &VectorcraftApp, ctx: &Context) -> bool {
    app.synthetic_frame || app.ui.answering_control || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::AccessKitActionRequest(_))))
}

/// The checklist's Start (or Resume) was pressed: arm the laser for this one command and run it.
/// The one place the app arms the laser (see the module docs). `automated`: the press (or a tick)
/// came from automation. The command's own refusal (not Idle, off the bed, …) shows in the status
/// bar.
pub fn confirm(app: &mut VectorcraftApp, list: &mut Checklist, automated: bool) -> Result<Value, Refused> {
    if !list.complete() {
        return Err(Refused::Unticked);
    }
    // Read again now: the port can't have changed since the box was drawn without this seeing it.
    let st = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session));
    if !st.connected {
        return Err(Refused::NotConnected);
    }
    if !may_arm(automated || list.automated_ticks(), st.sim) {
        list.clear_automated_ticks();
        list.press = None;
        app.status(tl!("Only a person at this computer can start the laser: scripts, agents and accessibility clients can't."));
        return Err(Refused::NotAPerson);
    }
    if let Err(why) = still_the_job(app, list) {
        app.status(why);
        return Err(Refused::Changed);
    }
    let (cmd, params) = match list.kind {
        Confirm::Start => ("laser.start", list.params.clone()),
        Confirm::Resume => ("laser.resume", json!({})),
    };
    match app.session.laser_execute_armed(cmd, &params) {
        Ok(v) => Ok(v),
        Err(e) => {
            app.status(e.to_string());
            Ok(Value::Null)
        }
    }
}

/// Run motion command `id` (home, jog, unlock, set origin, a beam-off frame) for the person at the
/// Laser window: the engine takes these on a real machine only through this path. Automated input
/// gets them on the simulator only, as arming.
pub fn as_person(app: &mut VectorcraftApp, automated: bool, id: &str, params: Value) -> Result<Value, String> {
    let sim = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session)).sim;
    if !may_arm(automated, sim) {
        let why = tl!("Only a person at this computer can move a real laser: scripts, agents and accessibility clients can't.");
        app.status(why);
        return Err(why.to_string());
    }
    let r = app.session.laser_execute_person(id, &params).map_err(|e| e.to_string());
    match &r {
        Err(e) => app.status(e.clone()),
        Ok(_) => super::note_motion(app),
    }
    r
}

/// Draw the open checklist (a window over the app, not modal: the Laser window's Stop stays
/// clickable); closes it on Start, Cancel or Escape.
pub fn checklist(app: &mut VectorcraftApp, ctx: &Context, st: &Status) {
    let Some(mut list) = app.picabia.laser.checklist.take() else { return };
    // The job ended or the link closed under it: nothing to confirm.
    if !st.connected || (list.kind == Confirm::Resume && !st.paused()) {
        return;
    }
    let auto_now = automated_frame(app, ctx);
    let t = Tokens::get(ctx);
    let (mut go, mut cancel) = (false, false);
    let mut open = true;
    egui::Window::new(list.kind.title(list.knife))
        .id(Id::unique("picabia_laser_checklist"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(18)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(380.0);
            for line in &list.summary {
                ui.label(egui::RichText::new(line).size(12.0).color(t.text));
            }
            if !list.summary.is_empty() {
                ui.add_space(6.0);
            }
            let intro = match list.kind {
                Confirm::Start if list.knife => tl!("The blade can cut the material, the bed and fingers. Tick each box once it's true."),
                Confirm::Resume if list.knife => tl!("Resuming lowers the blade and moves the head again."),
                Confirm::Start => tl!("The laser can burn the material, the machine and eyes. Tick each box once it's true."),
                Confirm::Resume => tl!("Resuming turns the beam back on."),
            };
            wrapped(ui, intro, t.text_dim);
            ui.add_space(8.0);
            for (i, text) in list.kind.checks(list.knife).into_iter().enumerate() {
                let on = list.ticks.get(i).copied().unwrap_or(false);
                if widgets::check(ui, text, on, true) {
                    list.toggle(i, auto_now);
                }
            }
            if !st.sim {
                ui.add_space(4.0);
                wrapped(ui, tl!("This is a real machine: only a click at this computer starts it."), readable(t.warning, &t));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let resp = widgets::primary_button_enabled(ui, list.kind.button(), list.complete());
                // Where the press came from, recorded when the button goes down: a press injected
                // and released later (a lost release the app completes) stays automated.
                if resp.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                    list.press = Some(auto_now);
                }
                go = resp.clicked() && list.complete();
                cancel = button(ui, tl!("Cancel"), false, true);
            });
        });
    let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if !open || cancel || escape {
        return;
    }
    if go {
        let automated = press_automated(list.press, auto_now);
        // A refusal says why in the status bar; the list stays open for a person's ticks (or, the
        // job changed, to be confirmed again with its new numbers).
        if !matches!(confirm(app, &mut list, automated), Err(Refused::NotAPerson | Refused::Changed)) {
            return;
        }
    }
    app.picabia.laser.checklist = Some(list);
}

/// The job's progress, how it ended, and Pause / Resume (what was pressed).
pub fn progress(ui: &mut Ui, st: &Status) -> Option<Action> {
    let t = Tokens::get(ui.ctx());
    let job = st.job.clone()?;
    ui.label(egui::RichText::new(&job.name).font(semibold(12.0)).color(t.text));
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 8.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.input);
    let done = rect.with_max_x(rect.min.x + rect.width() * (job.percent / 100.0) as f32);
    let fill = match job.state.as_str() {
        "failed" | "stopped" => t.error,
        "paused" => t.warning,
        _ => t.accent,
    };
    ui.painter().rect_filled(done, CornerRadius::same(4), fill);
    crate::picabia_ui::note(ui, &status::progress_line(&job));
    if let Some(outcome) = status::job_outcome(&job) {
        let c = match job.state.as_str() {
            "failed" => t.error,
            "done" => t.text,
            _ => t.text_dim,
        };
        wrapped(ui, &outcome, c);
    }
    let mut out = None;
    if job.active() {
        ui.horizontal(|ui| {
            if job.state == "paused" {
                if button(ui, tl!("Resume…"), true, !st.stopping) {
                    out = Some(Action::Resume);
                }
            } else if button(ui, tl!("Pause"), false, !st.stopping) {
                out = Some(Action::Pause);
            }
        });
    }
    out
}

/// The big red Stop: always there while connected. Whether it was pressed.
pub fn stop_button(ui: &mut Ui, h: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let label = tl!("Stop");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if resp.hovered() { t.error.gamma_multiply(0.85) } else { t.error };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let big = h >= 36.0;
    ui.painter().text(
        rect.center() - vec2(0.0, if big { 6.0 } else { 0.0 }),
        Align2::CENTER_CENTER,
        label,
        semibold(if big { 16.0 } else { 12.5 }),
        Color32::WHITE,
    );
    if big {
        ui.painter().text(
            rect.center() + vec2(0.0, 10.0),
            Align2::CENTER_CENTER,
            tl!("Shift+Esc anywhere"),
            egui::FontId::proportional(10.5),
            Color32::WHITE,
        );
    }
    resp.clicked()
}

/// Stop the laser now: the stop flag first (acted on by the link's own thread, whatever this one
/// does next), then `laser.stop`, which confirms it.
pub fn stop_now(app: &mut VectorcraftApp) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(h) = app.session.laser_stop_handle() {
        h.stop();
    }
    // An unconfirmed stop is an error the status bar shows; the link keeps trying.
    let _ = app.run("laser.stop", json!({}));
}

/// While connected with the window closed: a bar above the bottom of the window with the
/// machine's state, the job's progress, Stop and a way back to the window.
pub fn bar(app: &mut VectorcraftApp, ctx: &Context, st: &Status) {
    let t = Tokens::get(ctx);
    let (badge, colour) = status::badge(st, &t);
    let (mut stop, mut show) = (false, false);
    egui::Area::new(Id::unique("picabia_laser_bar")).order(egui::Order::Foreground).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -48.0)).show(ctx, |ui| {
        egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Laser")).font(semibold(12.0)).color(t.text));
                ui.label(egui::RichText::new(&badge).font(semibold(12.0)).color(readable(colour, &t)));
                if let Some(j) = st.job.as_ref().filter(|j| j.active()) {
                    crate::picabia_ui::note(ui, &format!("{} · {}%", j.name, crate::picabia_ui::num(j.percent.round())));
                }
                ui.allocate_ui(vec2(84.0, 28.0), |ui| stop = stop_button(ui, 28.0));
                show = button(ui, tl!("Show"), false, true);
            });
        });
    });
    if stop {
        stop_now(app);
    }
    if show {
        app.picabia.laser.open = true;
    }
}

/// Shift+Esc: the emergency stop, anywhere in the app (a field, a menu, a dialog, a floating
/// panel's own window, the Laser window closed), whenever the link is open, whatever its status
/// says. Runs before the shortcuts and dialogs see the key (seams in `handle_input_once` and the
/// floating panels' body). Esc alone never stops a job.
pub fn stop_shortcut(app: &mut VectorcraftApp, ctx: &Context) {
    if !super::link_open(app) {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Escape)) {
        stop_now(app);
        app.status(tl!("Laser stopped"));
    }
}
