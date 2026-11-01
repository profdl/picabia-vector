//! PICABIA: running a laser job from the Laser window: the safety checklist that Start opens (and
//! the short confirm a paused job's Resume opens), the one place the app arms the laser, the job's
//! progress, and the emergency stop (the big Stop button, the bar that stays on screen while the
//! window is closed, and Shift+Esc anywhere in the app).
//!
//! # Arming
//! [`vectorcraft_engine::Session::laser_execute_armed`] is called from [`confirm`] only, after every
//! box of the checklist is ticked (a test checks that no other file of the app calls it). The
//! checklist is UI state, not a dialog: `ui.dialog.confirm`, commands, MCP and scripts can't reach
//! it. Clicks the control channel injects (`ui.click`, `ui.key`: a frame of synthetic input) or a
//! control request being answered arm only the simulator: on a real machine they are refused, so
//! scripts and agents can't fire the laser through the window either.
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
    /// The boxes to tick, in order.
    pub fn checks(self) -> Vec<&'static str> {
        match self {
            Confirm::Start => vec![
                tl!("The laser shielding is in place"),
                tl!("Everyone near the machine wears laser safety glasses"),
                tl!("The laser is focused for this material"),
                tl!("Extraction or ventilation is on"),
                tl!("The material lies flat and is held down"),
                tl!("Someone stays at the machine until the job ends"),
            ],
            Confirm::Resume => {
                vec![tl!("The shielding is in place and safety glasses are on"), tl!("Nothing has moved and someone is at the machine")]
            }
        }
    }

    fn title(self) -> &'static str {
        match self {
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

/// The open checklist: what it confirms, the job's params (for Start) and the boxes ticked.
#[derive(Clone, Debug)]
pub struct Checklist {
    pub kind: Confirm,
    pub params: Value,
    pub ticks: Vec<bool>,
}

impl Checklist {
    pub fn new(kind: Confirm, params: Value) -> Self {
        Checklist { kind, params, ticks: vec![false; kind.checks().len()] }
    }

    /// Every box is ticked.
    pub fn complete(&self) -> bool {
        !self.ticks.is_empty() && self.ticks.iter().all(|t| *t)
    }
}

/// Why [`confirm`] didn't run the job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not every box is ticked.
    Unticked,
    /// The click came from the control channel (or a control request is being answered) and the
    /// machine is a real one.
    NotAPerson,
    /// The link closed meanwhile.
    NotConnected,
}

/// Whether a confirmed checklist may arm the laser: always for a person's click, and for automated
/// input (see [`automated`]) only on the simulator.
pub fn may_arm(automated: bool, sim: bool) -> bool {
    !automated || sim
}

/// Whether the input acted on now came from automation: a frame carrying input the control
/// channel injected, or a control request being answered.
pub fn automated(app: &VectorcraftApp) -> bool {
    app.synthetic_frame || app.ui.answering_control
}

/// The checklist's Start (or Resume) was pressed: arm the laser for this one command and run it.
/// The one place the app arms the laser (see the module docs). The command's own refusal (not
/// Idle, off the bed, …) shows in the status bar.
pub fn confirm(app: &mut VectorcraftApp, list: &Checklist) -> Result<Value, Refused> {
    if !list.complete() {
        return Err(Refused::Unticked);
    }
    // Read again now: the port can't have changed since the box was drawn without this seeing it.
    let st = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session));
    if !st.connected {
        return Err(Refused::NotConnected);
    }
    if !may_arm(automated(app), st.sim) {
        app.status(tl!("Only a person at this computer can start the laser: scripts and agents can't."));
        return Err(Refused::NotAPerson);
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

/// Draw the open checklist (a window over the app, not modal: the Laser window's Stop stays
/// clickable); closes it on Start, Cancel or Escape.
pub fn checklist(app: &mut VectorcraftApp, ctx: &Context, st: &Status) {
    let Some(mut list) = app.picabia.laser.checklist.take() else { return };
    // The job ended or the link closed under it: nothing to confirm.
    if !st.connected || (list.kind == Confirm::Resume && !st.paused()) {
        return;
    }
    let t = Tokens::get(ctx);
    let (mut go, mut cancel) = (false, false);
    let mut open = true;
    egui::Window::new(list.kind.title())
        .id(Id::unique("picabia_laser_checklist"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(18)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(360.0);
            let intro = match list.kind {
                Confirm::Start => tl!("The laser can burn the material, the machine and eyes. Tick each box once it's true."),
                Confirm::Resume => tl!("Resuming turns the beam back on."),
            };
            wrapped(ui, intro, t.text_dim);
            ui.add_space(8.0);
            for (i, text) in list.kind.checks().into_iter().enumerate() {
                let on = list.ticks.get(i).copied().unwrap_or(false);
                if widgets::check(ui, text, on, true)
                    && let Some(tick) = list.ticks.get_mut(i)
                {
                    *tick = !on;
                }
            }
            if !st.sim {
                ui.add_space(4.0);
                wrapped(ui, tl!("This is a real machine: only a click at this computer starts it."), readable(t.warning, &t));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                go = button(ui, list.kind.button(), true, list.complete());
                cancel = button(ui, tl!("Cancel"), false, true);
            });
        });
    let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if !open || cancel || escape {
        return;
    }
    if go && list.complete() {
        // A refusal says why in the status bar (or the box isn't ticked).
        let _ = confirm(app, &list);
        return;
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

/// Shift+Esc: the emergency stop, anywhere in the app (a field, a menu, a dialog, the Laser window
/// closed), whenever a job runs or the machine moves. Runs before the shortcuts and dialogs see the
/// key (seam in `handle_input_once`). Esc alone never stops a job.
pub fn stop_shortcut(app: &mut VectorcraftApp, ctx: &Context) {
    if !super::link_open(app) {
        return;
    }
    let st = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session));
    if st.stoppable() && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Escape)) {
        stop_now(app);
        app.status(tl!("Laser stopped"));
    }
}
