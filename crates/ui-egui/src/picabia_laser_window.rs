//! PICABIA: the Laser window: connect to a GRBL laser (or the simulator), see its state, home,
//! unlock and jog it, set the job's origin, frame the job, choose what to cut and how, start it
//! through the safety checklist, follow its progress, pause, resume and stop it, and edit the
//! machine profile. A floating window opened by `laser.panel` (File ▸ Laser…, the command
//! palette); not part of the modifier UI. Every action is a `laser.*` command, except the one
//! armed start ([`run::confirm`]); the window reads `laser.status` each frame while open or
//! connected, repainting about ten times a second while connected.
//!
//! Safety (see [`run`]): Start opens the checklist and only its Start arms the laser; a large red
//! Stop is always on screen while connected (in the window, or in a bar at the bottom of the app
//! when the window is closed) and Shift+Esc stops from anywhere. Esc never stops a job and doesn't
//! close this window; closing it with a job running leaves the bar and its Stop.

pub mod job;
pub mod ladder;
pub mod ops;
mod profile;
pub mod run;
pub mod status;

use egui::{Context, CornerRadius, Id, Sense, Ui, vec2};
use serde_json::{Value, json};

pub use job::{JobState, Source};
pub use run::{Checklist, Confirm, stop_shortcut};
pub use status::Status;

use crate::VectorcraftApp;
use crate::i18n;
use crate::picabia_ui::{button, num, readable, wrapped};
use crate::theme::{Tokens, semibold};
use crate::widgets;

/// The label column of the window's rows.
pub(crate) const LABEL_W: f32 = 76.0;
/// Jog steps, mm.
pub const STEPS: [f64; 4] = [0.1, 1.0, 10.0, 100.0];
/// A column's width.
const COLUMN: f32 = 372.0;
/// Between the columns.
const GUTTER: f32 = 24.0;
/// The window's content width.
const WIDTH: f32 = COLUMN * 2.0 + GUTTER + 1.0;
/// The environment variable that turns real laser ports off (`0`, `off`, `false`, `no`: only the
/// simulator) or on under automation (`1`, `on`, `true`, `yes`). Agents and screenshots run the app
/// with `0`.
pub const HARDWARE_ENV: &str = "PICABIA_LASER_HARDWARE";

/// Whether the desktop app turns real laser ports on (seam in the app's `main`). The Laser window,
/// with its always-visible Stop and Shift+Esc, is what makes them safe to offer. Off when
/// [`HARDWARE_ENV`] says no, and off whenever the app answers a control channel (`controlled`:
/// `--control`, which agents and MCP drive) unless [`HARDWARE_ENV`] says yes explicitly.
pub fn hardware_allowed(controlled: bool) -> bool {
    hardware_allowed_by(std::env::var(HARDWARE_ENV).ok().as_deref(), controlled)
}

/// [`hardware_allowed`] for the variable's value `v`.
pub fn hardware_allowed_by(v: Option<&str>, controlled: bool) -> bool {
    match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "off" | "false" | "no") => false,
        Some("1" | "on" | "true" | "yes") => true,
        _ => !controlled,
    }
}

/// Why a file dialog doesn't open while the laser works.
pub const STOP_FIRST: &str = "Stop or finish the laser job first: a file dialog would cover the Stop button";

/// Whether a file dialog must not open now: the laser is working (a job, a jog, homing, a frame),
/// and a native file dialog would hold the window, its Stop button and Shift+Esc (seam in
/// `picks::ask`, which every file dialog goes through). Says so in the status bar.
pub fn refuse_file_dialog(app: &mut VectorcraftApp) -> bool {
    if !link_open(app) {
        return false;
    }
    let st = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session));
    if st.stoppable() || moved_lately(app) {
        app.status(tl!(STOP_FIRST));
        return true;
    }
    false
}

/// How long after a motion command is sent the machine counts as working, before the status says.
pub const MOTION_GRACE_SECS: f64 = 1.0;

/// A motion command went to the machine (from [`run::as_person`]).
pub(crate) fn note_motion(app: &mut VectorcraftApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.picabia.laser.last_motion = Some(std::time::Instant::now());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
}

/// A motion command was sent less than [`MOTION_GRACE_SECS`] ago.
fn moved_lately(app: &VectorcraftApp) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.picabia.laser.last_motion.is_some_and(|t| t.elapsed().as_secs_f64() < MOTION_GRACE_SECS)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        false
    }
}

/// The laser link is open (connected or connecting).
pub(crate) fn link_open(app: &VectorcraftApp) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.session.laser_stop_handle().is_some()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        false
    }
}

/// The Laser window's state (UI state, not the document).
#[derive(Clone, Debug)]
pub struct LaserPanel {
    pub open: bool,
    /// `laser.ports`: (path, name).
    pub ports: Vec<(String, String)>,
    ports_loaded: bool,
    /// The port picked (a path from `ports`).
    pub port: Option<String>,
    /// Jog step, mm.
    pub step: f64,
    /// Jog feed, mm/min (`None`: the profile's travel speed).
    pub feed: Option<f64>,
    pub job: JobState,
    pub checklist: Option<Checklist>,
    pub show_profile: bool,
    pub show_log: bool,
    /// `laser.profile`'s reply while the profile section is open (asked again after a change).
    profile: Option<Value>,
    /// The material the profile's table shows.
    pub profile_material: String,
    /// What the last action has to say (unlocking without homing, a frame's placement).
    pub notice: Option<String>,
    /// Where the last pointer press over the app came from while the window was up (`Some(true)`:
    /// automation), recorded when it went down: the motion buttons judge their click by it.
    pub last_press: Option<bool>,
    /// When a motion command (home, jog, unlock, set origin, frame) was last sent: the status takes
    /// a moment to say the machine moves, and meanwhile it counts as working.
    #[cfg(not(target_arch = "wasm32"))]
    pub last_motion: Option<std::time::Instant>,
}

impl Default for LaserPanel {
    fn default() -> Self {
        LaserPanel {
            open: false,
            ports: Vec::new(),
            ports_loaded: false,
            port: None,
            step: 10.0,
            feed: None,
            job: JobState::default(),
            checklist: None,
            show_profile: false,
            show_log: false,
            profile: None,
            profile_material: "paper".into(),
            notice: None,
            last_press: None,
            #[cfg(not(target_arch = "wasm32"))]
            last_motion: None,
        }
    }
}

/// What the window asks for, run once it has drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    RefreshPorts,
    Connect(String),
    Disconnect,
    Home,
    Unlock,
    Jog(f64, f64),
    /// Up (+) or down (−) by this many mm.
    JogZ(f64),
    JogCancel,
    SetOrigin,
    Frame(Value),
    /// Open the checklist for these job params.
    Start(Value),
    Pause,
    /// Open the short confirm for a paused job.
    Resume,
    Stop,
    Profile(Value),
    /// The selection's G-code export dialog.
    Export,
    /// Pick the G-code file to run.
    OpenFile,
}

/// Open the window (`laser.panel`).
pub fn open(app: &mut VectorcraftApp) {
    let p = &mut app.picabia.laser;
    p.open = true;
    p.ports_loaded = false;
}

/// The window, the checklist, and the bar while the window is closed (from `picabia_ui::show`).
pub fn show(app: &mut VectorcraftApp, ctx: &Context) {
    let link = link_open(app);
    if !app.picabia.laser.open && !link {
        return;
    }
    let st = Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session));
    if st.open() {
        // About ten times a second while connected: the position and the progress move.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    run::checklist(app, ctx, &st);
    if !app.picabia.laser.open {
        if st.open() {
            run::bar(app, ctx, &st);
        }
        return;
    }
    // Where a press comes from is judged when it goes down, not when its click lands.
    let auto_now = run::automated_frame(app, ctx);
    if ctx.input(|i| i.pointer.any_pressed()) {
        app.picabia.laser.last_press = Some(auto_now);
    }
    let mut panel = std::mem::take(&mut app.picabia.laser);
    if !panel.ports_loaded {
        load_ports(app, &mut panel);
    }
    let mut actions = Vec::new();
    let mut open = true;
    let t = Tokens::get(ctx);
    let room = ctx.content_rect();
    // While the machine works, over everything (a dialog's backdrop too): its Stop stays clickable.
    let order = if st.stoppable() { egui::Order::Foreground } else { egui::Order::Middle };
    egui::Window::new(tl!("Laser"))
        .id(Id::unique("picabia_laser_window"))
        .order(order)
        .collapsible(false)
        .resizable(false)
        // Near the top right, over the panels' side: clear of the toolbar on the left.
        .default_pos(room.right_top() + vec2(-WIDTH - 80.0, 72.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(14)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(WIDTH.min(room.width() - 60.0).max(320.0));
            if cfg!(target_arch = "wasm32") {
                wrapped(ui, tl!("Laser control needs the desktop app. G-code export works here: File ▸ Export G-code for Laser…"), t.text_dim);
                ui.add_space(8.0);
                if button(ui, "Export G-code…", true, true) {
                    actions.push(Action::Export);
                }
                return;
            }
            body(app, ui, &mut panel, &st, &mut actions);
        });
    // Laser… asked for again while the window drew keeps it open.
    let asked = std::mem::take(&mut app.picabia.laser).open;
    panel.open = open || asked;
    app.picabia.laser = panel;
    // Where the click's press came from: motion and arming on a real machine are a person's only.
    let automated = run::press_automated(app.picabia.laser.last_press, auto_now);
    for a in actions {
        act(app, a, &st, automated);
    }
}

/// The window's sections.
fn body(app: &mut VectorcraftApp, ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    connection(ui, panel, st, actions);
    if st.open() {
        ui.add_space(8.0);
        if run::stop_button(ui, 46.0) {
            actions.push(Action::Stop);
        }
    }
    ui.add_space(4.0);
    // Two columns: the machine on the left, the job on the right.
    ui.horizontal_top(|ui| {
        column(ui, |ui| machine_column(app, ui, panel, st, actions));
        ui.add_space(GUTTER / 2.0);
        let (r, _) = ui.allocate_exact_size(vec2(1.0, ui.available_height().max(1.0)), Sense::hover());
        ui.painter().vline(r.center().x, r.y_range(), egui::Stroke::new(1.0, Tokens::get(ui.ctx()).divider));
        ui.add_space(GUTTER / 2.0);
        column(ui, |ui| job_column(app, ui, panel, st, actions));
    });
}

/// One column of the window.
fn column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(COLUMN, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(COLUMN);
        add(ui);
    });
}

/// The machine: its state, jogging, the profile and the log.
fn machine_column(app: &mut VectorcraftApp, ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    section(ui, tl!("Machine"));
    machine(ui, st, actions);
    if let Some(n) = &panel.notice {
        wrapped(ui, &i18n::msg(n), readable(t.warning, &t));
    }
    if st.connected {
        section(ui, tl!("Position"));
        jog(ui, panel, st, actions);
    }
    ui.add_space(8.0);
    widgets::divider(ui);
    if widgets::section_toggle(ui, "Machine profile", panel.show_profile) {
        panel.show_profile = !panel.show_profile;
    }
    if panel.show_profile {
        let profile = panel.profile.get_or_insert_with(|| app.session.execute("laser.profile", &json!({})).unwrap_or(Value::Null)).clone();
        if let Some(set) = profile::rows(ui, &profile, st, &mut panel.profile_material) {
            actions.push(Action::Profile(set));
        }
    } else {
        panel.profile = None;
    }
    if widgets::section_toggle(ui, "Log", panel.show_log) {
        panel.show_log = !panel.show_log;
    }
    if panel.show_log {
        log(ui, st);
    }
}

/// The job: what is cut and how, Frame and Start, and its progress.
fn job_column(app: &mut VectorcraftApp, ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    section(ui, tl!("Job"));
    let now = ui.ctx().input(|i| i.time);
    let knife = st.knife_settings();
    if panel.job.knife != knife {
        // Another head writes another job: dry-run it again.
        panel.job.knife = knife;
        panel.job.changed(now);
    }
    if job::settings(app, ui, &mut panel.job, st.job_active()) {
        panel.job.changed(now);
    }
    if std::mem::take(&mut panel.job.want_file) {
        actions.push(Action::OpenFile);
    }
    // Nothing changes under a running job: no dry runs meanwhile.
    if !st.job_active() {
        job::refresh(app, ui.ctx(), &mut panel.job);
    }
    ui.add_space(4.0);
    job::summary(ui, &panel.job);
    ui.add_space(8.0);
    start_row(app, ui, panel, st, actions);
    if st.job.is_some() {
        section(ui, tl!("Progress"));
        if let Some(a) = run::progress(ui, st) {
            actions.push(a);
        }
    }
}

/// A section's heading.
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    widgets::divider(ui);
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(title).font(semibold(12.0)).color(t.text_strong));
    ui.add_space(2.0);
}

/// A row's label.
pub(crate) fn lead(ui: &mut Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui(vec2(LABEL_W, 24.0), |ui| {
        ui.set_min_width(LABEL_W);
        ui.label(egui::RichText::new(label).size(12.0).color(t.text));
    });
}

fn load_ports(app: &mut VectorcraftApp, panel: &mut LaserPanel) {
    panel.ports_loaded = true;
    let r = app.session.execute("laser.ports", &json!({})).unwrap_or(Value::Null);
    panel.ports = r
        .get("ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|p| {
            let path = p.get("path")?.as_str()?.to_string();
            let name = p.get("name").and_then(Value::as_str).unwrap_or(&path).to_string();
            Some((path, name))
        })
        .collect();
    if panel.port.as_ref().is_none_or(|p| !panel.ports.iter().any(|(path, _)| path == p)) {
        panel.port = panel.ports.first().map(|(p, _)| p.clone());
    }
}

/// The port picker and Connect / Disconnect.
fn connection(ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        lead(ui, tl!("Port"));
        if st.open() {
            let name = st.port_name.clone().or_else(|| st.port.clone()).unwrap_or_default();
            ui.allocate_ui(vec2(240.0, 24.0), |ui| {
                ui.set_min_width(240.0);
                ui.label(egui::RichText::new(name).size(12.0).color(t.text));
            });
            if button(ui, "Disconnect", false, !st.job_active()) {
                actions.push(Action::Disconnect);
            }
            return;
        }
        let names: Vec<&str> = panel.ports.iter().map(|(_, n)| n.as_str()).chain(std::iter::once(tl!("Look for ports again"))).collect();
        let shown =
            panel.port.as_ref().and_then(|p| panel.ports.iter().find(|(path, _)| path == p)).map_or(tl!("No port found"), |(_, n)| n.as_str());
        if let Some(i) = widgets::dropdown_names_named(ui, "picabia_laser_port", Some(tl!("Port")), shown, &names, 240.0) {
            match panel.ports.get(i) {
                Some((path, _)) => panel.port = Some(path.clone()),
                None => actions.push(Action::RefreshPorts),
            }
        }
        if button(ui, "Connect", true, panel.port.is_some())
            && let Some(p) = &panel.port
        {
            actions.push(Action::Connect(p.clone()));
        }
    });
    if !st.open() && st.phase == "lost" {
        wrapped(ui, tl!("The connection was lost: check the cable and connect again."), t.error);
    }
    if !st.open() && !st.hardware && !cfg!(target_arch = "wasm32") {
        wrapped(ui, tl!("Real laser ports are off in this session: only the simulator can connect."), t.text_dim);
    }
}

/// The machine's state badge, position, Home / Unlock, its alarm and errors.
fn machine(ui: &mut Ui, st: &Status, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (text, colour) = status::badge(st, &t);
        let galley = ui.painter().layout_no_wrap(text, semibold(12.0), readable(colour, &t));
        let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 24.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(12), colour.gamma_multiply(0.16));
        ui.painter().circle_filled(rect.left_center() + vec2(11.0, 0.0), 4.0, colour);
        ui.painter().galley(rect.left_center() + vec2(20.0, -galley.size().y / 2.0), galley, readable(colour, &t));
        if let Some(p) = st.pos {
            let pos = format!("X {}  Y {}  mm", num(p[0]), num(p[1]));
            ui.label(egui::RichText::new(pos).font(crate::theme::mono(11.5)).color(t.text));
        }
    });
    if st.connected {
        let mut facts = Vec::new();
        if let Some(v) = &st.version {
            facts.push(format!("GRBL {v}"));
        }
        if let Some(s) = st.s_max_machine {
            facts.push(format!("$30 = {}", num(s)));
        }
        match st.laser_mode {
            Some(true) => facts.push(tl!("laser mode on").to_string()),
            Some(false) => facts.push(tl!("laser mode off ($32 = 0)").to_string()),
            None => {}
        }
        if st.sim {
            facts.push(tl!("simulated").to_string());
        }
        if !facts.is_empty() {
            crate::picabia_ui::note(ui, &facts.join(" · "));
        }
        ui.horizontal(|ui| {
            let free = !st.job_active();
            if button(ui, "Home", false, free) {
                actions.push(Action::Home);
            }
            if button(ui, "Unlock", false, free) {
                actions.push(Action::Unlock);
            }
        });
    }
    if let Some(a) = &st.alarm {
        ui.add_space(4.0);
        egui::Frame::NONE.fill(t.error.gamma_multiply(0.12)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(8)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                let text = i18n::fmt(tl!("Alarm {code}: {text}"), &[("code", &a.code.to_string()), ("text", &a.text)]);
                ui.label(egui::RichText::new(text).font(semibold(12.0)).color(t.error));
                if !a.advice.is_empty() {
                    wrapped(ui, &a.advice, t.text);
                }
            },
        );
    }
    if let Some(e) = &st.last_error {
        wrapped(ui, &i18n::msg(e), t.error);
    }
    for w in &st.warnings {
        wrapped(ui, &i18n::msg(w), readable(t.warning, &t));
    }
}

/// A jog arrow pointing `dir` (unit vector, screen space) in `r`.
fn arrow(ui: &Ui, r: egui::Rect, dir: egui::Vec2, color: egui::Color32) {
    let c = r.center();
    let (tip, side) = (c + dir * 6.0, egui::vec2(-dir.y, dir.x) * 5.0);
    let base = c - dir * 3.0;
    ui.painter().add(egui::Shape::convex_polygon(vec![tip, base + side, base - side], color, egui::Stroke::NONE));
}

/// The jog pad (arrows, step, feed) and Set origin.
fn jog(ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let free = st.connected && !st.job_active() && st.state.as_deref() != Some("Alarm");
    ui.horizontal(|ui| {
        // The pad: up, left / right, down, and the jog's stop in the middle.
        const B: f32 = 30.0;
        let (rect, _) = ui.allocate_exact_size(vec2(B * 3.0 + 8.0, B * 3.0 + 8.0), Sense::hover());
        let cell = |cx: f32, cy: f32| egui::Rect::from_min_size(rect.min + vec2(cx * (B + 4.0), cy * (B + 4.0)), vec2(B, B));
        let s = panel.step;
        for (r, dir, (dx, dy), name) in [
            (cell(1.0, 0.0), vec2(0.0, -1.0), (0.0, s), tl!("Jog up (Y+)")),
            (cell(0.0, 1.0), vec2(-1.0, 0.0), (-s, 0.0), tl!("Jog left (X−)")),
            (cell(2.0, 1.0), vec2(1.0, 0.0), (s, 0.0), tl!("Jog right (X+)")),
            (cell(1.0, 2.0), vec2(0.0, 1.0), (0.0, -s), tl!("Jog down (Y−)")),
        ] {
            let resp =
                ui.interact(r, Id::unique(("picabia_laser_jog", name)), if free { Sense::click() } else { Sense::hover() }).on_hover_text(name);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, free, name));
            let fill = if free && resp.hovered() { t.hover } else { t.button };
            ui.painter().rect_filled(r, CornerRadius::same(5), fill);
            arrow(ui, r, dir, if free { t.icon } else { t.text_disabled });
            if free && resp.clicked() {
                actions.push(Action::Jog(dx, dy));
            }
        }
        let mid = cell(1.0, 1.0);
        let name = tl!("Stop the jog");
        let resp =
            ui.interact(mid, Id::unique("picabia_laser_jog_cancel"), if st.connected { Sense::click() } else { Sense::hover() }).on_hover_text(name);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, st.connected, name));
        ui.painter().rect_stroke(mid.shrink(7.0), CornerRadius::same(3), egui::Stroke::new(1.5, t.icon), egui::StrokeKind::Inside);
        if st.connected && resp.clicked() {
            actions.push(Action::JogCancel);
        }
        // Z: up and down (the knife onto the sheet before Set origin, a laser's focus height), at
        // most 10 mm a click: a Z axis is short and the bed is under it.
        let (zr, _) = ui.allocate_exact_size(vec2(B, B * 3.0 + 8.0), Sense::hover());
        let s = s.min(10.0);
        for (r, dir, dz, name) in [
            (egui::Rect::from_min_size(zr.min, vec2(B, B)), vec2(0.0, -1.0), s, tl!("Raise the head (Z+)")),
            (egui::Rect::from_min_size(zr.min + vec2(0.0, 2.0 * (B + 4.0)), vec2(B, B)), vec2(0.0, 1.0), -s, tl!("Lower the head (Z−)")),
        ] {
            let resp =
                ui.interact(r, Id::unique(("picabia_laser_jog_z", name)), if free { Sense::click() } else { Sense::hover() }).on_hover_text(name);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, free, name));
            let fill = if free && resp.hovered() { t.hover } else { t.button };
            ui.painter().rect_filled(r, CornerRadius::same(5), fill);
            arrow(ui, r, dir, if free { t.icon } else { t.text_disabled });
            if free && resp.clicked() {
                actions.push(Action::JogZ(dz));
            }
        }
        ui.painter().text(zr.center(), egui::Align2::CENTER_CENTER, "Z", egui::FontId::proportional(12.0), t.text_dim);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Step")).size(12.0).color(t.text));
                let labels = ["0.1", "1", "10", "100"];
                let shown = STEPS.iter().position(|v| (*v - panel.step).abs() < 1e-9).and_then(|i| labels.get(i)).copied().unwrap_or("10");
                if let Some(i) = widgets::dropdown_names_named(ui, "picabia_laser_step", Some(tl!("Step")), shown, &labels, 70.0) {
                    panel.step = STEPS.get(i).copied().unwrap_or(10.0);
                }
                crate::picabia_ui::note(ui, "mm");
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Feed")).size(12.0).color(t.text));
                let rapid = st.profile.get("rapidMmMin").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(3000.0);
                if let Some(v) =
                    widgets::plain_field_named(ui, "picabia_laser_feed", Some(tl!("Feed")), panel.feed.unwrap_or(rapid), "mm/min", 0, 120.0)
                    && v.is_finite()
                {
                    panel.feed = Some(v.clamp(1.0, 20_000.0));
                }
            });
            ui.add_space(4.0);
            if button(ui, "Set origin here", false, free) {
                actions.push(Action::SetOrigin);
            }
        });
    });
    if st.knife() {
        crate::picabia_ui::note(
            ui,
            tl!(
                "Jog to the sheet's bottom-left corner, lower the blade until it just touches the sheet, Set origin (it zeroes Z too), Frame, then Start."
            ),
        );
        if st.connected && !st.knife_z_set {
            wrapped(ui, tl!("Z isn't zeroed for the knife on this connection yet."), readable(t.warning, &t));
        }
    } else {
        crate::picabia_ui::note(ui, tl!("Jog to the sheet's bottom-left corner, Set origin, Frame, then Start."));
    }
    crate::picabia_ui::note(
        ui,
        tl!("While a menu of the menu bar is open the app can't react, Shift+Esc and Stop included: use the machine's emergency stop."),
    );
}

/// Frame, Start…, and why Start can't run when it can't.
fn start_row(app: &mut VectorcraftApp, ui: &mut Ui, panel: &mut LaserPanel, st: &Status, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let sel = job::selection(app);
    let params = panel.job.params(&sel);
    let mut blocked: Vec<String> = Vec::new();
    match &panel.job.summary {
        job::Summary::Failed(_) => blocked.push(tl!("the job can't be cut as it is (see above)").to_string()),
        job::Summary::Pending => blocked.push(tl!("the job is still being worked out").to_string()),
        _ => {}
    }
    if params.is_none() {
        blocked.push(match panel.job.source {
            Source::File(_) => tl!("no G-code file chosen").to_string(),
            _ => tl!("nothing selected").to_string(),
        });
    }
    if st.job_active() {
        blocked.push(tl!("a job is running").to_string());
    } else {
        blocked.extend(st.blocked_by.iter().map(|b| i18n::msg(b)));
    }
    let can_frame = params.is_some() && st.connected && !st.job_active() && st.state.as_deref() == Some("Idle");
    let can_start = blocked.is_empty() && st.can_start;
    ui.horizontal(|ui| {
        if button(ui, "Frame", false, can_frame)
            && let Some(p) = &params
        {
            actions.push(Action::Frame(p.clone()));
        }
        if button(ui, "Start…", true, can_start)
            && let Some(p) = &params
        {
            actions.push(Action::Start(p.clone()));
        }
        if button(ui, "Export G-code…", false, matches!(panel.job.source, Source::Selection) && !sel.is_empty()) {
            actions.push(Action::Export);
        }
    });
    if !can_start && !blocked.is_empty() {
        wrapped(ui, &i18n::fmt(tl!("Start needs: {reasons}"), &[("reasons", &blocked.join("; "))]), t.text_dim);
    }
}

/// The last lines to and from the machine.
fn log(ui: &mut Ui, st: &Status) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.fill(t.input).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("picabia_laser_log").max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
            if st.log.is_empty() {
                crate::picabia_ui::note(ui, tl!("Nothing yet"));
            }
            for l in &st.log {
                ui.label(egui::RichText::new(l).font(crate::theme::mono(10.5)).color(t.text_dim));
            }
        });
    });
}

/// The warnings a command result lists, as one line (`None`: none).
fn warnings_of(r: &Result<Value, String>) -> Option<String> {
    let w: Vec<String> = r.as_ref().ok().map(|r| crate::picabia_ui::strings(r, "warnings", 8)).unwrap_or_default();
    (!w.is_empty()).then(|| w.join(" "))
}

/// Run what the window asked for.
fn act(app: &mut VectorcraftApp, a: Action, st: &Status, automated: bool) {
    let empty = json!({});
    match a {
        Action::RefreshPorts => app.picabia.laser.ports_loaded = false,
        Action::Connect(port) => {
            app.picabia.laser.notice = None;
            let _ = app.run("laser.connect", json!({"port": port}));
        }
        Action::Disconnect => {
            let _ = app.run("laser.disconnect", empty);
        }
        Action::Home => {
            app.picabia.laser.notice = None;
            let _ = run::as_person(app, automated, "laser.home", empty);
        }
        Action::Unlock => {
            let r = run::as_person(app, automated, "laser.unlock", empty);
            app.picabia.laser.notice = r.ok().and_then(|r| r.get("warning").and_then(Value::as_str).map(str::to_string));
        }
        Action::Jog(dx, dy) => {
            let mut p = json!({});
            if dx != 0.0 {
                p["dx"] = json!(dx);
            }
            if dy != 0.0 {
                p["dy"] = json!(dy);
            }
            if let Some(f) = app.picabia.laser.feed {
                p["feed"] = json!(f);
            }
            let _ = run::as_person(app, automated, "laser.jog", p);
        }
        Action::JogZ(dz) => {
            let mut p = json!({"dz": dz});
            if let Some(f) = app.picabia.laser.feed {
                p["feed"] = json!(f);
            }
            let _ = run::as_person(app, automated, "laser.jog", p);
        }
        Action::JogCancel => {
            let _ = app.run("laser.jogCancel", empty);
        }
        Action::SetOrigin => {
            app.picabia.laser.notice = None;
            let _ = run::as_person(app, automated, "laser.setOrigin", empty);
        }
        Action::Frame(p) => {
            let r = run::as_person(app, automated, "laser.frame", p);
            app.picabia.laser.notice = warnings_of(&r);
        }
        Action::Start(p) => {
            let job = &app.picabia.laser.job;
            let start = job.start_params(p);
            let knife = job.knife.is_some();
            let mut list = Checklist::for_head(Confirm::Start, start.clone(), knife);
            list.summary = job.checklist_lines(&job::selection_name(app).unwrap_or_default());
            list.expected = job.expected(&job::selection(app), &start);
            app.picabia.laser.checklist = Some(list);
        }
        Action::Pause => {
            let _ = app.run("laser.pause", empty);
        }
        Action::Resume => {
            if st.paused() {
                let knife = app.picabia.laser.job.knife.is_some();
                app.picabia.laser.checklist = Some(Checklist::for_head(Confirm::Resume, Value::Null, knife));
            } else {
                // A hold that isn't a job's (a jog): resuming it lights nothing.
                let _ = app.run("laser.resume", empty);
            }
        }
        Action::Stop => run::stop_now(app),
        Action::Profile(set) => {
            profile::write(app, set);
            app.picabia.laser.profile = None;
        }
        Action::Export => {
            let ids: Vec<u64> = job::selection(app).iter().map(|id| id.0).collect();
            let _ = app.run("laser.gcode", json!({"ids": ids}));
        }
        // A file dialog would hold the window, and its Stop, while it is open.
        Action::OpenFile if st.stoppable() => app.status(tl!(STOP_FIRST)),
        Action::OpenFile => crate::picks::button(app, |app| {
            let pick = crate::FilePick {
                name: String::new(),
                folder: None,
                filters: crate::FilePick::filters([("G-code", &["gcode", "nc", "ngc", "gc", "tap", "txt"][..])]),
            };
            if let Some(path) = crate::picks::open(app, &pick) {
                let j = &mut app.picabia.laser.job;
                j.source = Source::File(path);
                j.summary = job::Summary::File;
            }
        }),
    }
}

#[cfg(test)]
mod tests;
