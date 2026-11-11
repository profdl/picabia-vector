//! PICABIA: the Laser window against the simulated GRBL machine only (no test opens, lists or
//! connects a real port: the app here never enables hardware, so `laser.ports` lists only the
//! simulator).

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::run::{self, Checklist, Confirm, Refused};
use super::status::{self, Status};
use super::*;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app
}

fn status_of(app: &VectorcraftApp) -> Status {
    Status::from_json(&vectorcraft_engine::cmd::picabia_laser_link::status_json(&app.session))
}

/// Poll the link until `done`.
fn wait(app: &mut VectorcraftApp, what: &str, done: impl Fn(&Status) -> bool) -> Status {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let st = status_of(app);
        if done(&st) {
            return st;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: {st:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Connected to the simulator (`sim` its options), ready and Idle.
fn connect(app: &mut VectorcraftApp, sim: Value) {
    app.run("laser.connect", json!({"port": "sim", "sim": sim})).unwrap();
    wait(app, "ready", |s| s.phase == "ready" && s.state.as_deref() == Some("Idle"));
}

/// A selected 40 pt square.
fn square(app: &mut VectorcraftApp) -> u64 {
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    id
}

fn ticked(kind: Confirm, params: Value) -> Checklist {
    let mut c = Checklist::new(kind, params);
    c.ticks.iter_mut().for_each(|t| *t = true);
    c
}

/// A frame of the app's UI with `events` (fonts installed: the window draws text).
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
    let raw =
        egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        run::stop_shortcut(app, ui.ctx());
        crate::picabia_ui::show(app, ui.ctx());
    });
    out.textures_delta.clear();
    out.shapes
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> String {
    let mut out = String::new();
    for s in shapes {
        if let egui::Shape::Text(t) = &s.shape {
            out.push_str(t.galley.text());
            out.push('\n');
        }
    }
    out
}

fn context(dark: bool) -> egui::Context {
    crate::widgets::look_tests::themed(if dark { crate::theme::Brightness::Dark } else { crate::theme::Brightness::Light })
}

#[test]
fn hardware_stays_off_when_the_environment_says_so_or_a_control_channel_is_open() {
    assert!(hardware_allowed_by(None, false), "the desktop app on its own");
    assert!(!hardware_allowed_by(None, true), "--control: off");
    assert!(!hardware_allowed_by(Some("junk"), true));
    for on in ["1", " ON ", "true", "yes"] {
        assert!(hardware_allowed_by(Some(on), true), "{on}: on even under --control");
    }
    for off in ["0", "off", " FALSE ", "no"] {
        assert!(!hardware_allowed_by(Some(off), false), "{off}");
    }
}

#[test]
fn only_a_persons_click_arms_a_real_machine() {
    assert!(run::may_arm(false, false), "a person, a real machine");
    assert!(run::may_arm(false, true), "a person, the simulator");
    assert!(run::may_arm(true, true), "injected input, the simulator");
    assert!(!run::may_arm(true, false), "injected input never arms a real machine");
}

#[test]
fn only_the_checklist_arms_the_laser() {
    // The one call of the engine's arming method in the whole app is the checklist's confirm.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src];
    let mut callers = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("picabia_laser_window/tests.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains(".laser_execute_armed(") {
                    callers.push(p.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().to_string_lossy().into_owned());
                }
            }
        }
    }
    assert_eq!(callers, ["src/picabia_laser_window/run.rs"]);
    let run = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/picabia_laser_window/run.rs")).unwrap();
    assert_eq!(run.matches(".laser_execute_armed(").count(), 1, "once, in confirm");
}

#[test]
fn the_window_lists_only_the_simulator_and_says_real_ports_are_off() {
    let mut app = app();
    app.run(crate::picabia_ui::LASER_PANEL, json!({})).unwrap();
    for dark in [false, true] {
        let ctx = context(dark);
        frame(&mut app, &ctx, vec![]);
        let text = texts(&frame(&mut app, &ctx, vec![]));
        assert!(text.contains("Laser"), "{text}");
        assert!(text.contains("Real laser ports are off"), "{text}");
        assert!(!text.contains("Stop\n"), "no Stop before connecting: {text}");
    }
    assert!(app.picabia.laser.ports.iter().all(|(p, _)| p == "sim"), "{:?}", app.picabia.laser.ports);
}

#[test]
fn a_simulated_job_runs_to_done_from_the_checklist() {
    let mut app = app();
    let id = square(&mut app);
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    // Unticked: nothing runs.
    let list = Checklist::new(Confirm::Start, json!({"ids": [id]}));
    let mut list = list;
    assert_eq!(run::confirm(&mut app, &mut list, false), Err(Refused::Unticked));
    // The control channel's click (injected input) arms the simulator.
    let r = run::confirm(&mut app, &mut ticked(Confirm::Start, json!({"ids": [id]})), true).unwrap();
    assert!(r["started"]["lines"].as_u64().unwrap() > 4, "{r}");
    let st = wait(&mut app, "done", |s| s.job.as_ref().is_some_and(|j| j.state == "done"));
    assert!(status::job_outcome(st.job.as_ref().unwrap()).unwrap().starts_with("Done in"));
    // Without the checklist the same start is refused (the command can't arm).
    assert!(app.run("laser.start", json!({"ids": [id]})).unwrap_err().contains("isn't armed"));
    // The window draws the finished job in both themes.
    app.picabia.laser.open = true;
    for dark in [false, true] {
        let ctx = context(dark);
        frame(&mut app, &ctx, vec![]);
        let text = texts(&frame(&mut app, &ctx, vec![]));
        for want in ["Stop", "Shift+Esc anywhere", "Idle", "Progress", "Done in"] {
            assert!(text.contains(want), "{want} in {text}");
        }
    }
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn shift_esc_stops_a_running_job_and_esc_alone_does_not() {
    let mut app = app();
    let id = square(&mut app);
    connect(&mut app, json!({"speedFactor": 2, "timeScale": 1}));
    run::confirm(&mut app, &mut ticked(Confirm::Start, json!({"ids": [id]})), false).unwrap();
    wait(&mut app, "running", |s| s.job_active() && s.state.as_deref() == Some("Run"));
    let ctx = context(false);
    let key = |shift: bool| egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE },
    };
    frame(&mut app, &ctx, vec![key(false)]);
    assert!(status_of(&app).job_active(), "Esc alone never stops a job");
    // The window is closed: the bar shows the Stop.
    let text = texts(&frame(&mut app, &ctx, vec![]));
    assert!(text.contains("Stop") && text.contains("Show"), "{text}");
    frame(&mut app, &ctx, vec![key(true)]);
    let st = wait(&mut app, "stopped", |s| s.job.as_ref().is_some_and(|j| j.state == "stopped") && !s.stopping);
    assert_eq!(st.job.unwrap().state, "stopped");
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn pause_then_resume_asks_again_and_stop_works_from_the_button() {
    let mut app = app();
    let id = square(&mut app);
    connect(&mut app, json!({"speedFactor": 2, "timeScale": 1}));
    run::confirm(&mut app, &mut ticked(Confirm::Start, json!({"ids": [id]})), false).unwrap();
    wait(&mut app, "running", |s| s.job_active());
    app.run("laser.pause", json!({})).unwrap();
    let st = wait(&mut app, "paused", |s| s.paused());
    // Resume opens the short confirm, which runs the armed resume.
    act(&mut app, Action::Resume, &st, false);
    let list = app.picabia.laser.checklist.clone().unwrap();
    assert_eq!(list.kind, Confirm::Resume);
    run::confirm(&mut app, &mut ticked(Confirm::Resume, Value::Null), false).unwrap();
    wait(&mut app, "running again", |s| s.job.as_ref().is_some_and(|j| j.state == "running"));
    act(&mut app, Action::Stop, &st, false);
    wait(&mut app, "stopped", |s| s.job.as_ref().is_some_and(|j| j.state == "stopped"));
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn junk_status_reads_as_nothing() {
    for junk in [json!(null), json!(3), json!({"job": 4, "pos": {"x": "a"}, "alarm": [], "startBlockedBy": [1, "x"], "log": "no"})] {
        let st = Status::from_json(&junk);
        assert!(!st.open() && st.job.is_none() && st.pos.is_none() && st.alarm.is_none());
    }
    assert_eq!(status::clock(f64::NAN), "0:00");
    assert_eq!(status::clock(3725.0), "1:02:05");
    assert_eq!(status::clock(61.4), "1:01");
}

#[test]
fn ticks_and_presses_from_automation_are_told_apart() {
    let mut list = Checklist::new(Confirm::Start, json!({}));
    assert!(Confirm::Start.checks(false).contains(&"A hardware emergency stop is in reach"));
    for i in 0..list.ticks.len() {
        list.toggle(i, i == 2);
    }
    assert!(list.complete() && list.automated_ticks(), "box 3 was ticked by automation");
    list.clear_automated_ticks();
    assert!(!list.complete() && !list.automated_ticks());
    list.toggle(2, false);
    assert!(list.complete() && !list.automated_ticks(), "a person ticked it again");
    // Automation unticks and ticks again: marked; a person unticks and ticks: clear.
    list.toggle(0, true);
    list.toggle(0, true);
    assert!(list.ticks[0] && list.automated_ticks());
    list.toggle(0, false);
    list.toggle(0, false);
    assert!(list.ticks[0] && !list.automated_ticks());
}

#[test]
fn an_accessibility_action_counts_as_automation() {
    let app = app();
    let ctx = context(false);
    let mut seen = Vec::new();
    for events in [
        vec![],
        vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_node: egui::accesskit::NodeId(1),
            target_tree: egui::accesskit::TreeId::ROOT,
            data: None,
        })],
    ] {
        let raw = egui::RawInput { events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| seen.push(run::automated_frame(&app, ui.ctx())));
        out.textures_delta.clear();
    }
    assert_eq!(seen, [false, true]);
}

#[test]
fn the_checklist_shows_the_job_start_runs() {
    let mut app = app();
    let id = square(&mut app);
    let mut j = JobState::default();
    j.take_result(Ok(
        json!({"material": "film", "files": [{"wMm": 14.1, "hMm": 14.1, "lines": 12}], "gcode": {"cut": {"powerPct": 80, "speedMmMin": 250}}}),
    ));
    let p = j.start_params(j.params(&[vectorcraft_doc::NodeId(id)]).unwrap());
    assert_eq!((p["material"].as_str(), p["cut"]["powerPct"].as_f64()), (Some("film"), Some(80.0)), "what is shown is what runs");
    let lines = j.checklist_lines("Rectangle");
    assert_eq!(lines[0], "Rectangle");
    assert!(lines.iter().any(|l| l.contains("14.1 × 14.1 mm") && l.contains("12 lines")), "{lines:?}");
    assert!(lines.iter().any(|l| l == "Cut: 80 %, 250 mm/min, 2×"), "{lines:?}");
    j.source = Source::File("/x/a.gcode".into());
    assert_eq!(j.start_params(json!({"path": "/x/a.gcode"})), json!({"path": "/x/a.gcode"}), "a file runs as it is");
}

#[test]
fn no_file_dialog_opens_over_a_working_laser_and_motion_goes_the_persons_way() {
    let mut app = app();
    let id = square(&mut app);
    app.services.pick_open = Some(Box::new(|_| Some("/tmp/x.gcode".into())));
    let pick = crate::FilePick::named("x.gcode");
    assert_eq!(crate::picks::open(&mut app, &pick).as_deref(), Some("/tmp/x.gcode"), "not connected: it opens");
    connect(&mut app, json!({"speedFactor": 2, "timeScale": 1}));
    // The person's path on the simulator, even from automation.
    run::as_person(&mut app, true, "laser.jog", json!({"dx": 1})).unwrap();
    wait(&mut app, "the jog done", |s| s.state.as_deref() == Some("Idle") && s.pos.is_some_and(|p| p[0] > 0.99));
    run::confirm(&mut app, &mut ticked(Confirm::Start, json!({"ids": [id]})), false).unwrap();
    wait(&mut app, "running", |s| s.job_active());
    assert_eq!(crate::picks::open(&mut app, &pick), None, "refused while the laser works");
    assert!(app.ui.status.contains("Stop or finish the laser job first"), "{}", app.ui.status);
    // Shift+Esc acts whenever the link is open.
    let ctx = context(false);
    let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| run::stop_shortcut(&mut app, ui.ctx()));
    out.textures_delta.clear();
    wait(&mut app, "stopped", |s| s.job.as_ref().is_some_and(|j| j.state == "stopped") && !s.stopping);
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn start_refuses_a_job_that_changed_since_the_checklist_opened() {
    let mut app = app();
    let id = square(&mut app);
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    let sel = [vectorcraft_doc::NodeId(id)];
    let mut j = JobState::default();
    j.take_result(app.session.execute("laser.gcode", &json!({"ids": [id], "dryRun": true})).map_err(|e| e.to_string()));
    let start = j.start_params(j.params(&sel).unwrap());
    let mut list = ticked(Confirm::Start, start.clone());
    list.summary = j.checklist_lines("Rectangle");
    list.expected = j.expected(&sel, &start);
    let before = list.expected.clone().unwrap();
    // The design grows after the checklist opened.
    app.run("object.setBounds", json!({"width": 80, "height": 80})).unwrap();
    assert_eq!(run::confirm(&mut app, &mut list, false), Err(Refused::Changed));
    assert!(app.ui.status.contains("The job changed"), "{}", app.ui.status);
    let after = list.expected.clone().unwrap();
    assert!(after.w_mm > before.w_mm * 1.9, "{before:?} → {after:?}");
    assert!(!list.complete(), "the boxes are to be ticked again");
    assert!(list.summary.contains(&job::size_text(after.w_mm, after.h_mm, after.lines)), "{:?}", list.summary);
    assert!(status_of(&app).job.is_none(), "nothing ran");
    // Confirmed again with the new numbers, it runs.
    list.ticks.iter_mut().for_each(|t| *t = true);
    run::confirm(&mut app, &mut list, false).unwrap();
    wait(&mut app, "done", |s| s.job.as_ref().is_some_and(|j| j.state == "done"));
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn a_press_counts_where_it_went_down() {
    assert!(!run::press_automated(Some(false), false), "a person's press and click");
    assert!(run::press_automated(Some(true), false), "an injected press released by the app later");
    assert!(run::press_automated(Some(false), true), "a click landing in an automated frame");
    assert!(run::press_automated(None, false), "no pointer press (AccessKit, keyboard)");
    // The checklist records its Start button's press in the frame it goes down in.
    let mut app = app();
    square(&mut app);
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    let mut list = Checklist::new(Confirm::Start, json!({}));
    list.ticks.iter_mut().for_each(|t| *t = true);
    // One box left unticked: the click can't start anything, so the press stays to be read.
    list.ticks[0] = false;
    app.picabia.laser.checklist = Some(list);
    let ctx = context(false);
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let start = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Start" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .expect("the checklist's Start");
    let press = |pressed| egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    app.synthetic_frame = true;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), press(true)]);
    app.synthetic_frame = false;
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.picabia.laser.checklist.as_ref().and_then(|l| l.press), Some(true), "recorded when it went down");
    // The window's motion buttons read the same: the last press while the window is up.
    app.picabia.laser.open = true;
    app.synthetic_frame = true;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), press(true)]);
    app.synthetic_frame = false;
    assert_eq!(app.picabia.laser.last_press, Some(true));
    frame(&mut app, &ctx, vec![press(false), egui::Event::PointerMoved(start), press(true)]);
    assert_eq!(app.picabia.laser.last_press, Some(false), "a person's press");
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn a_motion_just_sent_holds_file_dialogs_off() {
    let mut app = app();
    app.services.pick_open = Some(Box::new(|_| Some("/tmp/x.gcode".into())));
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    let pick = crate::FilePick::named("x.gcode");
    assert!(crate::picks::open(&mut app, &pick).is_some(), "idle: it opens");
    run::as_person(&mut app, false, "laser.jog", json!({"dx": 0.1})).unwrap();
    assert_eq!(crate::picks::open(&mut app, &pick), None, "within a second of the jog, before the status says it moves");
    app.picabia.laser.last_motion = Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
    wait(&mut app, "idle", |s| s.state.as_deref() == Some("Idle") && !s.stoppable());
    assert!(crate::picks::open(&mut app, &pick).is_some(), "a while later, idle: it opens");
    app.run("laser.disconnect", json!({})).unwrap();
}

#[test]
fn the_ladders_checklist_shows_its_grid_and_the_line_count_is_what_is_sent() {
    let mut app = app();
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    let mut j = JobState::default();
    j.source = Source::PowerLadder;
    let (cmd, p) = j.dry_run(&[]).unwrap();
    j.take_result(app.session.execute(cmd, &p).map_err(|e| e.to_string()));
    let lines = j.checklist_lines("");
    assert!(lines.iter().any(|l| l == "Squares at 20–100 % power (5 steps) × 150–1200 mm/min (4 steps), 1 pass each"), "{lines:?}");
    assert!(lines.iter().all(|l| !l.starts_with("Cut:") && !l.starts_with("Score:")), "the table's cut and score aren't the ladder's: {lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("Engrave:")), "its labels are engraved: {lines:?}");
    assert_eq!(job::ladder_text(&json!({"powersPct": "x"})), None, "junk: no line");
    // The dry run's line count is what the job sends.
    let shown = j.chosen_sheet().unwrap().lines;
    run::confirm(&mut app, &mut ticked(Confirm::Start, j.start_params(j.params(&[]).unwrap())), false).unwrap();
    let st = wait(&mut app, "done", |s| s.job.as_ref().is_some_and(|j| j.state == "done"));
    assert_eq!(st.job.unwrap().lines, shown);
    app.run("laser.disconnect", json!({})).unwrap();
}

/// PICABIA (PIC35): with the drag knife as the head the profile shows the knife's settings, the
/// job its knife line, the pad lowers Z, Set origin zeroes it, and Start opens the knife's
/// checklist, which runs the job.
#[test]
fn the_drag_knife_head_changes_the_profile_jog_pad_job_and_checklist() {
    let mut app = app();
    let id = square(&mut app);
    connect(&mut app, json!({"speedFactor": 1000, "timeScale": 100}));
    app.run("laser.profile", json!({"set": {"head": "knife"}})).unwrap();
    app.picabia.laser.open = true;
    app.picabia.laser.show_profile = true;
    let ctx = context(false);
    frame(&mut app, &ctx, vec![]);
    let text = texts(&frame(&mut app, &ctx, vec![]));
    for want in ["Drag knife", "Cut depth (Z)", "Plunge speed", "Blade offset", "lower the blade", "Z isn't zeroed", "Knife: cut at Z -0.5 mm"] {
        assert!(text.contains(want), "{want} in {text}");
    }
    assert!(!text.contains("Air assist"), "the laser's rows are hidden: {text}");
    let st = status_of(&app);
    act(&mut app, Action::JogZ(-1.0), &st, false);
    wait(&mut app, "Z down", |s| s.pos.is_some_and(|p| (p[2] + 1.0).abs() < 1e-6) && s.state.as_deref() == Some("Idle"));
    act(&mut app, Action::SetOrigin, &st, false);
    let st = wait(&mut app, "Z zeroed", |s| s.knife_z_set && s.pos.is_some_and(|p| p[2].abs() < 1e-6));
    assert!(st.can_start, "{st:?}");
    act(&mut app, Action::Start(json!({"ids": [id]})), &st, false);
    let mut list = app.picabia.laser.checklist.take().unwrap();
    assert!(list.knife && list.ticks.len() == Confirm::Start.checks(true).len());
    assert!(list.summary.iter().any(|l| l.starts_with("Knife: cut at Z")), "{:?}", list.summary);
    assert!(!Confirm::Start.checks(true).iter().any(|c| c.contains("glasses")));
    list.ticks.iter_mut().for_each(|t| *t = true);
    run::confirm(&mut app, &mut list, false).unwrap();
    let st = wait(&mut app, "done", |s| s.job.as_ref().is_some_and(|j| j.state == "done"));
    assert!(st.pos.is_some_and(|p| (p[2] - 5.0).abs() < 1e-6), "up at the end: {st:?}");
    app.run("laser.disconnect", json!({})).unwrap();
}
