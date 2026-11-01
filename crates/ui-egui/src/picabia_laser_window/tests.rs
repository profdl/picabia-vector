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
fn hardware_stays_off_when_the_environment_says_so() {
    assert!(hardware_allowed_by(None));
    assert!(hardware_allowed_by(Some("1")));
    for off in ["0", "off", " FALSE ", "no"] {
        assert!(!hardware_allowed_by(Some(off)), "{off}");
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
    assert_eq!(run::confirm(&mut app, &list), Err(Refused::Unticked));
    // The control channel's click (injected input) arms the simulator.
    app.synthetic_frame = true;
    let r = run::confirm(&mut app, &ticked(Confirm::Start, json!({"ids": [id]}))).unwrap();
    app.synthetic_frame = false;
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
    run::confirm(&mut app, &ticked(Confirm::Start, json!({"ids": [id]}))).unwrap();
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
    run::confirm(&mut app, &ticked(Confirm::Start, json!({"ids": [id]}))).unwrap();
    wait(&mut app, "running", |s| s.job_active());
    app.run("laser.pause", json!({})).unwrap();
    let st = wait(&mut app, "paused", |s| s.paused());
    // Resume opens the short confirm, which runs the armed resume.
    act(&mut app, Action::Resume, &st);
    let list = app.picabia.laser.checklist.clone().unwrap();
    assert_eq!(list.kind, Confirm::Resume);
    run::confirm(&mut app, &ticked(Confirm::Resume, Value::Null)).unwrap();
    wait(&mut app, "running again", |s| s.job.as_ref().is_some_and(|j| j.state == "running"));
    act(&mut app, Action::Stop, &st);
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
