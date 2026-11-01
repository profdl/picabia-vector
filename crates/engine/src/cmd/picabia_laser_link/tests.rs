//! PICABIA: the laser link (`laser.*`) against the simulated GRBL machine only: connecting, every
//! command, jobs to done with the beam never on while travelling, pause and resume, stop, alarms, a
//! pulled cable, the emergency stop, quitting, every refusal (not armed, `$32 = 0`, not Idle, off
//! the bed), arming refused from every channel, and real ports never listed or opened.
//!
//! No test here opens a real serial port: real ports are refused until the desktop app enables
//! them, and in the engine's tests even then (`native::open_serial`); the port lister is replaced
//! wherever real ports are enabled.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use kurbo::Rect;
use serde_json::{Value, json};

use super::{MachineFrame, NOT_ARMED, PortRow, check_gcode, check_line, check_placement, gcode_bounds};
use crate::{EngineError, Session};

/// Fast enough to finish a job in well under a second.
fn fast() -> Value {
    json!({"speedFactor": 1000, "timeScale": 100})
}

/// Slow enough that a job is still running when the test pauses or stops it.
fn slow() -> Value {
    json!({"speedFactor": 5, "timeScale": 1})
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn st(s: &mut Session) -> Value {
    ex(s, "laser.status", json!({}))
}

/// Poll `laser.status` until `done` (panics with the last status after `secs`).
fn wait(s: &mut Session, secs: f64, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    loop {
        let v = st(s);
        if done(&v) {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: {v:#}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Connected to the simulator with `sim` options, ready and Idle.
fn connected(sim: Value) -> Session {
    let mut s = Session::new();
    let r = ex(&mut s, "laser.connect", json!({"port": "sim", "sim": sim}));
    assert_eq!(r["port"], "sim");
    assert_eq!(r["sim"], true);
    wait(&mut s, 10.0, "ready", |v| v["phase"] == "ready" && v["state"] == "Idle");
    s
}

fn job_state(v: &Value) -> &str {
    v["job"]["state"].as_str().unwrap_or("")
}

fn disabled(r: crate::Result<Value>) -> String {
    match r {
        Err(EngineError::Disabled(_, why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A 50 mm square at 10, 10 burnt `times` times (the beam off on every travel), 200 mm a time.
fn squares(times: usize) -> String {
    let mut g = String::from("; squares\nG21 G90 M5\nG0 X10 Y10\nM4 S300\n");
    for _ in 0..times {
        g.push_str("G1 X60 Y10 F1200\nG1 X60 Y60\nG1 X10 Y60\nG1 X10 Y10\n");
    }
    g.push_str("M5\nG0 X0 Y0\nM5\n");
    g
}

/// A job: `times` squares as G-code text.
fn job(times: usize) -> Value {
    json!({"gcode": squares(times), "name": "Squares"})
}

/// Start `p`'s job the way the desktop app's Laser window does (armed for this call only).
fn start(s: &mut Session, p: Value) -> Value {
    s.laser_execute_armed("laser.start", &p).unwrap_or_else(|e| panic!("laser.start {p}: {e}"))
}

#[test]
fn connect_reads_the_machine_and_lists_only_the_simulator() {
    let mut s = Session::new();
    let ports = ex(&mut s, "laser.ports", json!({}));
    assert_eq!(ports["ports"], json!([{"path": "sim", "name": "Simulated GRBL", "usb": false, "sim": true}]), "no real port outside the desktop app");
    let v = st(&mut s);
    assert_eq!((v["connected"].as_bool(), v["phase"].as_str(), v["hardware"].as_bool()), (Some(false), Some("closed"), Some(false)));
    assert_eq!(v["canStart"], false);
    assert_eq!(v["job"], Value::Null);
    let mut s = connected(json!({"sMax": 1200, "speedFactor": 1000, "timeScale": 100}));
    let v = st(&mut s);
    assert_eq!(v["connected"], true);
    assert_eq!(v["settings"]["laserMode"], true);
    assert_eq!(v["settings"]["sMax"], 1200.0);
    assert_eq!(v["sMaxUsed"], 1200.0, "the machine's $30 wins");
    assert_eq!(v["profile"]["sMax"], 1000.0);
    assert!(v["settings"]["version"].as_str().unwrap().starts_with("1.1"), "{v}");
    assert_eq!(v["settings"]["maxTravel"], json!([584.2, 1219.2, 50.0]));
    assert_eq!(v["bedMm"], json!({"w": 584.2, "h": 1219.2}));
    assert_eq!(v["canStart"], true, "{v}");
    assert_eq!(v["startBlockedBy"], json!([]));
    assert_eq!(v["armed"], false);
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("$30 is 1200")), "{v}");
    assert!(v["log"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("$$")), "{v}");
    assert!(v["pos"]["x"].is_number() && v["simStats"]["overflows"] == 0, "{v}");
    // The G-code export reads the connected machine's $30 and travel.
    assert_eq!(s.picabia.laser_machine.s_max, Some(1200.0));
    assert_eq!(s.picabia.laser_machine.travel_x_mm, Some(584.2));
    assert_eq!(ex(&mut s, "laser.profile", json!({}))["sMaxMachine"], 1200.0);
    // The test card's G-code uses the connected machine's $30 too.
    let v = start(&mut s, json!({"testCard": "card", "sMax": 500}));
    assert!(v["started"]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("$30 = 1200")), "{v}");
    wait(&mut s, 30.0, "the card", |v| job_state(v) == "done");
    // A shorter travel than the profile's bed narrows the bed.
    let mut t = connected(json!({"maxTravelMm": [300, 200, 50]}));
    assert_eq!(st(&mut t)["bedMm"], json!({"w": 300.0, "h": 200.0}));
    // Disconnect.
    let v = ex(&mut s, "laser.disconnect", json!({}));
    assert_eq!((v["connected"].as_bool(), v["phase"].as_str()), (Some(false), Some("closed")));
    assert!(s.execute("laser.home", &json!({})).is_err());
    assert_eq!(s.picabia.laser_machine, Default::default(), "cleared on disconnect");
    assert_eq!(ex(&mut s, "laser.profile", json!({}))["sMaxMachine"], Value::Null);
    // Connecting again works.
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": fast()}));
    wait(&mut s, 10.0, "ready again", |v| v["phase"] == "ready");
}

static LISTED: AtomicUsize = AtomicUsize::new(0);

/// A port lister standing in for the computer's (counts its calls).
fn fake_ports() -> Vec<PortRow> {
    LISTED.fetch_add(1, Ordering::SeqCst);
    vec![PortRow { path: "/nonexistent/picabia-test-laser".into(), name: "Test laser (not a device)".into(), usb: true }]
}

#[test]
fn real_ports_are_refused_until_the_desktop_app_enables_them_and_never_opened_in_tests() {
    let mut s = Session::new();
    s.picabia.laser.set_port_lister(fake_ports);
    let before = LISTED.load(Ordering::SeqCst);
    // Not enabled: the lister isn't even asked, and a real port is refused before anything opens.
    let ports = ex(&mut s, "laser.ports", json!({}));
    assert_eq!(ports["ports"].as_array().unwrap().len(), 1, "{ports}");
    assert_eq!(LISTED.load(Ordering::SeqCst), before, "no port listing outside the desktop app");
    let why = disabled(s.execute("laser.connect", &json!({"port": "/nonexistent/picabia-test-laser"})));
    assert!(why.contains("only in the desktop app"), "{why}");
    assert_eq!(st(&mut s)["phase"], "closed");
    // A refused real port leaves an open simulator link as it was.
    let mut t = connected(fast());
    assert!(disabled(t.execute("laser.connect", &json!({"port": "COM3"}))).contains("only in the desktop app"));
    assert_eq!(st(&mut t)["phase"], "ready");
    // Enabled (as the desktop app does): the lister's ports come first, then the simulator; the
    // engine's tests still can't open one.
    s.laser_enable_hardware();
    let ports = ex(&mut s, "laser.ports", json!({}));
    assert_eq!(ports["ports"][0]["path"], "/nonexistent/picabia-test-laser");
    assert_eq!(ports["ports"][1]["path"], "sim");
    assert!(LISTED.load(Ordering::SeqCst) > before);
    assert!(disabled(s.execute("laser.connect", &json!({"port": "/nonexistent/picabia-test-laser"}))).contains("never open a real port"));
    let v = st(&mut s);
    assert_eq!((v["phase"].as_str(), v["hardware"].as_bool()), (Some("closed"), Some(true)));
}

#[test]
fn the_machine_commands() {
    let mut s = connected(json!({"speedFactor": 50, "homing": true, "timeScale": 10}));
    let v = ex(&mut s, "laser.jog", json!({"dx": 10, "dy": 5, "feed": 3000}));
    assert_eq!(v["sent"], json!(["$J=G91 G21 X10 Y5 F3000"]));
    wait(&mut s, 5.0, "the jog", |v| v["state"] == "Idle" && v["pos"]["x"] == 10.0 && v["pos"]["y"] == 5.0);
    ex(&mut s, "laser.jog", json!({"x": 40.5, "y": 30}));
    wait(&mut s, 5.0, "the absolute jog", |v| v["state"] == "Idle" && v["pos"]["x"] == 40.5 && v["pos"]["y"] == 30.0);
    let v = ex(&mut s, "laser.setOrigin", json!({}));
    assert_eq!(v["sent"], json!(["G10 L20 P1 X0 Y0"]));
    wait(&mut s, 5.0, "the new origin", |v| v["pos"]["x"] == 0.0 && v["pos"]["y"] == 0.0 && v["mpos"]["x"] == 40.5);
    // Jog cancel stops a long jog.
    ex(&mut s, "laser.jog", json!({"dx": 400, "feed": 100}));
    wait(&mut s, 5.0, "jogging", |v| v["state"] == "Jog");
    ex(&mut s, "laser.jogCancel", json!({}));
    let v = wait(&mut s, 5.0, "the cancelled jog", |v| v["state"] == "Idle");
    assert!(v["pos"]["x"].as_f64().unwrap() < 400.0, "{v}");
    // Home and unlock.
    let v = ex(&mut s, "laser.home", json!({}));
    assert_eq!(v["sent"], json!(["$H"]));
    wait(&mut s, 10.0, "homing", |v| v["state"] == "Idle" && v["log"].as_array().unwrap().iter().any(|l| l == "> $H"));
    let v = ex(&mut s, "laser.unlock", json!({}));
    assert_eq!(v["sent"], json!(["$X"]));
    assert!(v["warning"].as_str().unwrap().contains("Home it"), "{v}");
    // Frame (beam off): the box of the job, the beam never on; no arm needed.
    let v = ex(&mut s, "laser.frame", job(1));
    assert_eq!(v["beam"], false);
    assert_eq!(v["boundsMm"], json!({"x0": 0.0, "y0": 0.0, "x1": 60.0, "y1": 60.0}));
    assert_eq!(v["sent"][0], "M5");
    assert_eq!(v["sent"][3], "$J=G90 G21 X60 Y60 F3000");
    let v = wait(&mut s, 10.0, "the frame", |v| v["state"] == "Idle" && v["pos"]["x"] == 0.0 && v["simStats"]["movedMm"].as_f64().unwrap() > 240.0);
    assert_eq!(v["simStats"]["burnMm"], 0.0, "{v}");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    // Pause, stop and resume are harmless with no job.
    for cmd in ["laser.pause", "laser.resume", "laser.stop"] {
        ex(&mut s, cmd, json!({}));
    }
    wait(&mut s, 5.0, "back after the stop", |v| v["phase"] == "ready" && !v["stopping"].as_bool().unwrap() && v["state"] == "Idle");
    // Junk.
    for (cmd, p) in [
        ("laser.jog", json!({"dx": 1, "x": 1})),
        ("laser.jog", json!({})),
        ("laser.jog", json!({"dx": "far"})),
        ("laser.jog", json!({"dx": 1e9})),
        ("laser.jog", json!({"dx": 1, "feed": 0})),
        ("laser.jog", json!({"dx": 1, "feed": "fast"})),
        ("laser.frame", json!({"beam": "yes"})),
        ("laser.frame", json!({"gcode": "G0 X1", "testCard": "card"})),
        ("laser.connect", json!({"port": 5})),
        ("laser.connect", json!({"port": ""})),
        ("laser.connect", json!({"port": "sim", "baud": 7})),
        ("laser.connect", json!({"port": "/dev/x", "sim": {}})),
        ("laser.connect", json!({"port": "sim", "sim": 3})),
    ] {
        assert!(matches!(s.execute(cmd, &p), Err(EngineError::BadParams { .. })), "{cmd} {p}");
    }
}

#[test]
fn a_job_runs_to_done_with_the_beam_off_while_travelling() {
    let mut s = connected(fast());
    let v = start(&mut s, job(2));
    let started = &v["started"];
    assert_eq!(started["name"], "Squares");
    assert_eq!(started["lines"], 14, "the comment line is dropped");
    assert_eq!(started["boundsMm"], json!({"x0": 0.0, "y0": 0.0, "x1": 60.0, "y1": 60.0}));
    assert_eq!(v["armed"], false, "the start used the arm up");
    let v = wait(&mut s, 30.0, "the job to finish", |v| job_state(v) == "done");
    let job = &v["job"];
    assert_eq!(job["percent"], 100.0);
    assert_eq!(job["line"], job["lines"]);
    assert_eq!(job["source"], "gcode");
    assert_eq!(job["etaSec"], 0.0);
    let sim = &v["simStats"];
    assert!((sim["burnMm"].as_f64().unwrap() - 400.0).abs() < 1e-6, "{sim}");
    assert_eq!(sim["beamOnTravelMm"], 0.0, "{sim}");
    assert_eq!(sim["beamOnStillMs"], 0.0, "{sim}");
    assert_eq!(sim["overflows"], 0);
    assert!(sim["maxRxUsed"].as_u64().unwrap() <= 128);
    assert_eq!(v["state"], "Idle");
    assert_eq!(v["canStart"], true, "{v}");
    // The test card and the power ladder.
    let v = start(&mut s, json!({"testCard": "card", "material": "film"}));
    assert!(v["started"]["estimatedSec"].as_f64().unwrap() > 0.0, "{v}");
    let v = wait(&mut s, 30.0, "the card", |v| job_state(v) == "done");
    assert_eq!(v["job"]["source"], "testCard");
    assert_eq!(v["job"]["name"], "Test card film");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    start(&mut s, json!({"testCard": "powerLadder", "powersPct": [20, 40], "speedsMmMin": [600]}));
    let v = wait(&mut s, 30.0, "the ladder", |v| job_state(v) == "done");
    assert_eq!(v["job"]["source"], "powerLadder");
    assert_eq!(v["job"]["name"], "Power ladder paper");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    // A custom material's card needs a stencil's limits.
    assert!(s.laser_execute_armed("laser.start", &json!({"testCard": "card", "material": "custom"})).is_err());
}

#[test]
fn a_gcode_file_runs_and_one_that_leaves_the_bed_is_refused() {
    let mut s = connected(fast());
    let dir = std::env::temp_dir().join(format!("picabia-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("square.gcode");
    std::fs::write(&ok, squares(1)).unwrap();
    let v = start(&mut s, json!({"path": ok.to_string_lossy()}));
    assert_eq!(v["started"]["name"], "square.gcode");
    assert_eq!(v["started"]["boundsMm"], json!({"x0": 0.0, "y0": 0.0, "x1": 60.0, "y1": 60.0}));
    let v = wait(&mut s, 20.0, "the file", |v| job_state(v) == "done");
    assert!((v["simStats"]["burnMm"].as_f64().unwrap() - 200.0).abs() < 1e-6, "{v}");
    assert_eq!(v["job"]["source"], "file");
    // Off the bed (Y 2000 on a 1219.2 mm bed).
    let far = dir.join("far.nc");
    std::fs::write(&far, "G21 G90\nG0 X10 Y2000\nM5\n").unwrap();
    let why = disabled(s.laser_execute_armed("laser.start", &json!({"path": far.to_string_lossy()})));
    assert!(why.contains("bigger than the bed"), "{why}");
    // Files Picabia can't check, and junk.
    let g92 = dir.join("g92.gcode");
    std::fs::write(&g92, "G92 X0 Y0\nG0 X1 Y1\n").unwrap();
    assert!(disabled(s.laser_execute_armed("laser.start", &json!({"path": g92.to_string_lossy()}))).contains("G92"));
    let svg = dir.join("x.svg").to_string_lossy().to_string();
    assert!(matches!(s.laser_execute_armed("laser.start", &json!({"path": svg})), Err(EngineError::BadParams { .. })));
    assert!(s.laser_execute_armed("laser.start", &json!({"path": dir.join("missing.gcode").to_string_lossy()})).is_err());
    let bin = dir.join("bin.gcode");
    std::fs::write(&bin, [0xffu8, 0xfe, 0x00, 0x47]).unwrap();
    assert!(disabled(s.laser_execute_armed("laser.start", &json!({"path": bin.to_string_lossy()}))).contains("plain text"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pause_and_resume() {
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    let v = ex(&mut s, "laser.pause", json!({}));
    assert_eq!(job_state(&v), "paused");
    wait(&mut s, 5.0, "the hold", |v| v["state"] == "Hold");
    std::thread::sleep(Duration::from_millis(150));
    let held = wait(&mut s, 5.0, "the hold", |v| v["state"] == "Hold");
    let moved = held["simStats"]["movedMm"].as_f64().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let later = st(&mut s);
    assert_eq!(later["simStats"]["movedMm"].as_f64().unwrap(), moved, "nothing moves in a hold");
    assert_eq!(later["simStats"]["beamOnStillMs"], 0.0, "the beam is off in a hold");
    // Resuming a paused job lights the beam again: only the desktop app's window can.
    let why = disabled(s.execute("laser.resume", &json!({})));
    assert!(why.contains("isn't armed"), "{why}");
    assert_eq!(job_state(&st(&mut s)), "paused");
    let v = s.laser_execute_armed("laser.resume", &json!({})).unwrap();
    assert_eq!(job_state(&v), "running");
    assert_eq!(v["armed"], false);
    wait(&mut s, 5.0, "moving again", |v| v["simStats"]["movedMm"].as_f64().unwrap() > moved + 1.0);
    ex(&mut s, "laser.stop", json!({}));
}

#[test]
fn stop_mid_job() {
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    let v = ex(&mut s, "laser.stop", json!({}));
    assert_eq!(job_state(&v), "stopped");
    let v = wait(&mut s, 5.0, "the stop sequence", |v| !v["stopping"].as_bool().unwrap() && v["state"] == "Idle");
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1, "{v}");
    assert_eq!(v["alarm"], Value::Null, "stopped before the reset: no alarm 3");
    assert_eq!(v["power"], 0.0, "{v}");
    assert!(v["log"].as_array().unwrap().iter().any(|l| l == "> M5"), "M5 after the reset: {v}");
    let burnt = v["simStats"]["burnMm"].as_f64().unwrap();
    assert!(burnt < 4000.0, "stopped before the end: {v}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(st(&mut s)["simStats"]["burnMm"].as_f64().unwrap(), burnt, "nothing burns after the stop");
    assert_eq!(st(&mut s)["simStats"]["beamOnTravelMm"], 0.0);
    // The next job may start.
    start(&mut s, job(20));
    assert_eq!(job_state(&st(&mut s)), "running");
    // Stop works from inside a command batch too (any channel).
    ex(&mut s, "file.new", json!({"width": 300, "height": 300}));
    ex(&mut s, "command.batch", json!({"commands": [{"command": "laser.stop", "params": {}}]}));
    wait(&mut s, 5.0, "the batch's stop", |v| job_state(v) == "stopped");
}

#[test]
fn an_alarm_mid_job_fails_it_at_its_line() {
    let mut s = connected(json!({"speedFactor": 1000, "timeScale": 100, "alarmAtLine": 30}));
    start(&mut s, job(20));
    let v = wait(&mut s, 20.0, "the alarm", |v| job_state(v) == "failed" && !v["stopping"].as_bool().unwrap() && v["state"] == "Alarm");
    assert!(v["job"]["why"].as_str().unwrap().contains("alarm 1"), "{v}");
    assert!(v["job"]["failedLine"].as_u64().unwrap() > 0);
    assert_eq!(v["alarm"]["code"], 1, "{v}");
    assert!(v["alarm"]["advice"].as_str().unwrap().len() > 10);
    assert_eq!(v["canStart"], false);
    assert!(v["startBlockedBy"][0].as_str().unwrap().contains("alarm"), "{v}");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    assert!(disabled(s.laser_execute_armed("laser.start", &job(1))).contains("alarm"));
    // An error at a line fails it too.
    let mut s = connected(json!({"speedFactor": 1000, "timeScale": 100, "errorAtLine": 12}));
    start(&mut s, job(20));
    let v = wait(&mut s, 20.0, "the error", |v| job_state(v) == "failed");
    assert!(v["job"]["why"].as_str().unwrap().contains("error 20"), "{v}");
    assert!(v["lastError"].as_str().unwrap().contains("The job stopped"), "{v}");
}

#[test]
fn a_pulled_cable_fails_the_job_and_loses_the_link() {
    let mut s = connected(json!({"speedFactor": 1000, "timeScale": 100, "disconnectAtLine": 25}));
    start(&mut s, job(20));
    let v = wait(&mut s, 20.0, "the lost port", |v| v["phase"] == "lost");
    assert_eq!(v["connected"], false);
    assert_eq!(job_state(&v), "failed");
    assert!(v["job"]["why"].as_str().unwrap().contains("lost"), "{v}");
    assert!(v["lastError"].as_str().unwrap().contains("cable"), "{v}");
    assert!(disabled(s.execute("laser.home", &json!({}))).contains("isn't connected"));
    // Stop and pause stay harmless; connecting again works.
    ex(&mut s, "laser.stop", json!({}));
    ex(&mut s, "laser.pause", json!({}));
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": fast()}));
    let v = wait(&mut s, 10.0, "ready again", |v| v["phase"] == "ready");
    assert_eq!(v["job"], Value::Null, "a new connection starts without a job");
}

#[test]
fn disconnect_mid_job_stops_it_first() {
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    let t = Instant::now();
    let v = ex(&mut s, "laser.disconnect", json!({}));
    assert!(t.elapsed() < Duration::from_millis(1200), "{:?}", t.elapsed());
    assert_eq!(v["connected"], false);
    assert_eq!(job_state(&v), "stopped");
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1, "the machine was reset (beam off) before the port closed: {v}");
    assert!(v["log"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("stop")), "{v}");
    assert!(disabled(s.execute("laser.jog", &json!({"dx": 1}))).contains("isn't connected"));
}

#[test]
fn the_emergency_stop_and_a_panicking_command_stop_a_job() {
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    s.laser_emergency_stop();
    let v = wait(&mut s, 5.0, "the stop", |v| job_state(v) == "stopped" && !v["stopping"].as_bool().unwrap() && v["state"] == "Idle");
    assert_eq!(v["armed"], false);
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1);
    // A command that panics stops the job through run_guarded.
    start(&mut s, job(20));
    wait(&mut s, 10.0, "running", |v| job_state(v) == "running");
    let r = s.run_guarded("test.panic", |_| panic!("boom"));
    assert!(matches!(r, Err(EngineError::Internal { .. })), "{r:?}");
    let v = wait(&mut s, 5.0, "the stop", |v| job_state(v) == "stopped");
    assert_eq!(v["connected"], true, "the link stays open");
    // A command that panics while armed leaves the session disarmed.
    s.picabia.laser.armed = true;
    let _ = s.run_guarded("test.panic", |_| panic!("boom"));
    assert_eq!(st(&mut s)["armed"], false);
}

#[test]
fn quitting_and_dropping_stop_a_job_quickly() {
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    let t = Instant::now();
    s.laser_shutdown();
    let took = t.elapsed();
    assert!(took < Duration::from_millis(700), "{took:?}");
    let v = st(&mut s);
    assert_eq!(v["connected"], false);
    assert_eq!(job_state(&v), "stopped");
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1, "{v}");
    // Dropping the session does the same, as fast.
    let mut s = connected(slow());
    start(&mut s, job(20));
    wait(&mut s, 10.0, "running", |v| job_state(v) == "running");
    let t = Instant::now();
    drop(s);
    assert!(t.elapsed() < Duration::from_millis(700), "{:?}", t.elapsed());
}

#[test]
fn arming_is_refused_from_every_channel_but_the_desktop_window() {
    let mut s = connected(fast());
    // Everything that isn't the desktop app reaches the engine through execute: the control
    // channel, MCP, the CLI, actions and batches. None of it can arm, whatever the params say.
    for p in [job(1), json!({"gcode": squares(1), "armed": true}), json!({"gcode": squares(1), "arm": true})] {
        let r = s.execute("laser.start", &p);
        assert!(matches!(&r, Err(EngineError::Disabled(_, why)) if why == NOT_ARMED) || matches!(&r, Err(EngineError::BadParams { .. })), "{r:?}");
    }
    let r = s.execute("command.batch", &json!({"commands": [{"command": "laser.start", "params": job(1)}]}));
    assert!(r.is_err(), "{r:?}");
    assert!(disabled(s.execute("laser.frame", &json!({"gcode": squares(1), "beam": true}))).contains("isn't armed"));
    assert_eq!(st(&mut s)["job"], Value::Null, "nothing started");
    assert_eq!(st(&mut s)["simStats"]["burnMm"], 0.0);
    // No laser command leaves the session armed, nor does arming outlive its one call.
    for id in super::specs().iter().map(|c| c.id).filter(|id| *id != "laser.disconnect") {
        let _ = s.execute(id, &json!({}));
        assert!(!s.picabia.laser.armed, "{id}");
    }
    assert!(s.laser_execute_armed("laser.start", &json!({"gcode": "junk"})).is_err());
    assert!(!s.picabia.laser.armed, "a refused start uses the arm up");
    assert_eq!(disabled(s.execute("laser.start", &job(1))), NOT_ARMED);
    // Only the beam commands run armed.
    for id in ["laser.connect", "laser.jog", "command.batch", "laser.stop"] {
        assert!(disabled(s.laser_execute_armed(id, &json!({}))).contains("only laser.start"), "{id}");
        assert!(!s.picabia.laser.armed);
    }
    // A beam frame from the window: 1% of $30 (once the stop above is over).
    wait(&mut s, 5.0, "back after the stop", |v| v["phase"] == "ready" && v["stopping"] == false && v["state"] == "Idle");
    let v = s.laser_execute_armed("laser.frame", &json!({"gcode": squares(1), "beam": true})).unwrap();
    assert!(v["sent"].as_array().unwrap().iter().any(|l| l == "M4 S10"), "1% of $30 = 1000: {v}");
    let v = wait(&mut s, 20.0, "the beam frame", |v| job_state(v) == "done");
    assert_eq!(v["job"]["source"], "frame");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
}

/// The one way to arm, [`Session::laser_execute_armed`], is called only by the engine's own laser
/// code and its tests, and by the desktop app's Laser window when it lands
/// (`crates/ui-egui/src/picabia_laser_panel*`): never by the control channel, MCP, the CLI or a
/// plug-in host.
#[test]
fn only_the_laser_window_may_arm() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut hits = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("apps")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if name.ends_with(".rs") && std::fs::read_to_string(&p).unwrap().contains("laser_execute_armed(") {
                hits.push(p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let allowed = |f: &str| f.starts_with("crates/engine/src/cmd/picabia_laser_link") || f.starts_with("crates/ui-egui/src/picabia_laser_panel");
    let others: Vec<&String> = hits.iter().filter(|f| !allowed(f)).collect();
    assert!(others.is_empty(), "laser_execute_armed is called from {others:?}");
    assert!(hits.iter().any(|f| f.ends_with("picabia_laser_link/tests.rs")), "{hits:?}");
}

#[test]
fn laser_mode_off_and_not_idle_are_refused() {
    let mut s = connected(json!({"laserMode": false, "speedFactor": 1000, "timeScale": 100}));
    let v = st(&mut s);
    assert_eq!(v["settings"]["laserMode"], false);
    assert_eq!(v["canStart"], false);
    assert!(v["startBlockedBy"][0].as_str().unwrap().contains("$32=1"), "{v}");
    let why = disabled(s.laser_execute_armed("laser.start", &json!({"testCard": "card"})));
    assert!(why.contains("Laser mode is off") && why.contains("$32=1"), "{why}");
    // A beam frame too; a plain frame is fine (the beam stays off).
    assert!(disabled(s.laser_execute_armed("laser.frame", &json!({"testCard": "card", "beam": true}))).contains("$32=1"));
    ex(&mut s, "laser.frame", json!({"testCard": "card"}));
    let v = wait(&mut s, 10.0, "the frame", |v| v["state"] == "Idle" && v["simStats"]["movedMm"].as_f64().unwrap() > 100.0);
    assert_eq!(v["simStats"]["burnMm"], 0.0);
    // In alarm.
    let mut s = Session::new();
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": {"startInAlarm": true, "homing": true, "speedFactor": 1000, "timeScale": 100}}));
    let v = wait(&mut s, 10.0, "ready in alarm", |v| v["phase"] == "ready" && v["state"] == "Alarm");
    assert_eq!(v["canStart"], false);
    assert!(disabled(s.laser_execute_armed("laser.start", &json!({"testCard": "card"}))).contains("alarm"));
    // Home, then it may start.
    ex(&mut s, "laser.home", json!({}));
    wait(&mut s, 10.0, "homed", |v| v["state"] == "Idle");
    start(&mut s, json!({"testCard": "card"}));
    wait(&mut s, 30.0, "the card", |v| job_state(v) == "done");
    // A job running: no second start, no jog, no reconnect.
    let mut s = connected(slow());
    start(&mut s, job(20));
    assert!(disabled(s.laser_execute_armed("laser.start", &job(1))).contains("job is running"));
    assert!(disabled(s.execute("laser.jog", &json!({"dx": 1}))).contains("job is running"));
    assert!(disabled(s.execute("laser.connect", &json!({"port": "sim"}))).contains("job is running"));
    ex(&mut s, "laser.stop", json!({}));
}

#[test]
fn not_connected_refuses_and_stop_is_harmless() {
    let mut s = Session::new();
    assert!(disabled(s.laser_execute_armed("laser.start", &json!({"testCard": "card"}))).contains("isn't connected"));
    assert!(disabled(s.execute("laser.frame", &json!({"testCard": "card"}))).contains("isn't connected"));
    for id in ["laser.stop", "laser.pause", "laser.disconnect", "laser.status"] {
        assert_eq!(ex(&mut s, id, json!({}))["connected"], false, "{id}");
    }
    for id in ["laser.home", "laser.unlock", "laser.setOrigin", "laser.jogCancel"] {
        assert!(disabled(s.execute(id, &json!({}))).contains("isn't connected"), "{id}");
    }
}

#[test]
fn junk_params_on_every_laser_command() {
    let junk = [
        json!({}),
        Value::Null,
        json!([1, 2]),
        json!("sim"),
        json!({"port": "sim", "sim": {"laserMode": "x", "sMax": -1e308, "speedFactor": 1e308, "timeScale": -5, "alarmAtLine": 1e300, "maxTravelMm": [1e308, "x", -1]}}),
        json!({"port": "/nonexistent/picabia-no-such-port", "baud": 1e308}),
        json!({"port": "/nonexistent/picabia-no-such-port", "baud": 9600}),
        json!({"dx": 1e308, "dy": "NaN", "feed": -1}),
        json!({"x": [1], "y": {"a": 1}, "feed": 1e-308}),
        json!({"beam": 1, "testCard": 5, "gcode": 7}),
        json!({"gcode": "G0 X1e400 Y\u{0}", "name": [1]}),
        json!({"gcode": "G91\nG0 X5\n", "beam": true}),
        json!({"gcode": "(unclosed\nG2 X1 Y1 R1e308\n"}),
        json!({"gcode": "G1 X-5 Y-5\n"}),
        json!({"testCard": "card", "material": [1], "sMax": "x", "cut": 5, "engrave": {"passes": 1e308}, "originXMm": 1e308, "kerfMm": -1}),
        json!({"testCard": "powerLadder", "powersPct": [1e308, -1, "x"], "speedsMmMin": [], "squareMm": -1, "gapMm": 1e308, "passes": 1e9}),
        json!({"path": 5}),
        json!({"path": "\u{0}.gcode"}),
        json!({"path": "/nonexistent/picabia/x.gcode", "gcode": "G0 X1"}),
    ];
    let ids: Vec<&str> = super::specs().iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), 14, "{ids:?}");
    assert!(ids.iter().all(|id| crate::command_specs().iter().any(|c| c.id == *id)), "registered");
    for (connect, hardware) in [(false, false), (true, false), (false, true)] {
        let mut s = if connect { connected(fast()) } else { Session::new() };
        s.picabia.laser.set_port_lister(fake_ports);
        if hardware {
            s.laser_enable_hardware();
        }
        for cmd in &ids {
            for j in &junk {
                for armed in [false, true] {
                    let r = if armed && super::BEAM_COMMANDS.contains(cmd) { s.laser_execute_armed(cmd, j) } else { s.execute(cmd, j) };
                    if let Err(EngineError::Internal { cmd, msg }) = r {
                        panic!("{cmd} panicked on {j}: {msg}");
                    }
                    assert!(!s.picabia.laser.armed);
                }
            }
            // A junk connect never left the simulator for a real port.
            let v = st(&mut s);
            assert!(v["port"].is_null() || v["port"] == "sim", "{cmd}: {v}");
        }
        let _ = s.execute("laser.stop", &json!({}));
    }
}

/// A long job through `laser.start` against the simulator at a high speed: wall time, lines per
/// second, and how long `laser.status` takes meanwhile.
#[test]
fn a_100k_line_job_streams_and_status_stays_fast() {
    let mut s = connected(json!({"speedFactor": 100000, "timeScale": 1000}));
    let mut text = String::from("G21\nG90\nM5\nM4 S100\n");
    for i in 0..33_332 {
        let y = (i % 100) as f64;
        text.push_str(&format!("G0 X10 Y{y}\nG1 X20 Y{y} F3000\nG1 X10 Y{y}\n"));
    }
    text.push_str("M5\n");
    let t = Instant::now();
    let v = start(&mut s, json!({"gcode": text}));
    let lines = v["started"]["lines"].as_u64().unwrap();
    assert!(lines > 100_000, "{lines}");
    let mut slowest = Duration::ZERO;
    let mut polls = 0u32;
    loop {
        let q = Instant::now();
        let v = st(&mut s);
        slowest = slowest.max(q.elapsed());
        polls += 1;
        if job_state(&v) == "done" {
            assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
            assert_eq!(v["simStats"]["overflows"], 0);
            break;
        }
        assert!(job_state(&v) == "running", "{v}");
        assert!(t.elapsed() < Duration::from_secs(120), "{v}");
        std::thread::sleep(Duration::from_millis(5));
    }
    let wall = t.elapsed().as_secs_f64();
    eprintln!(
        "TIMING laser.start 100k: {lines} lines in {wall:.2} s wall = {:.0} lines/s; laser.status slowest {:.3} ms over {polls} polls",
        lines as f64 / wall,
        slowest.as_secs_f64() * 1000.0
    );
    assert!(slowest < Duration::from_millis(20), "laser.status took {slowest:?}");
}

#[test]
fn placement_with_the_origin_at_the_head() {
    let job = Rect::new(0.0, 0.0, 330.1, 180.1);
    let bed = (584.2, 1219.2);
    // Nothing known: only the size, with a warning.
    let w = check_placement(job, bed, true, &MachineFrame::default()).unwrap();
    assert!(w[0].contains("only its size"), "{w:?}");
    assert!(check_placement(Rect::new(0.0, 0.0, 600.0, 10.0), bed, true, &MachineFrame::default()).unwrap_err().contains("bigger than the bed"));
    // Homed, positive space (the head at +100, +50), origin set at machine 100, 50: fits.
    let pos = MachineFrame { wco: Some([100.0, 50.0]), mpos: Some([100.0, 50.0]), travel: [Some(584.2), Some(1219.2)], homing: true };
    assert_eq!(check_placement(job, bed, true, &pos).unwrap(), Vec::<String>::new());
    // Origin at machine X 300: the sheet would reach 630.1.
    let far = MachineFrame { wco: Some([300.0, 0.0]), mpos: Some([300.0, 10.0]), ..pos };
    let e = check_placement(job, bed, true, &far).unwrap_err();
    assert!(e.contains("machine X 300 to 630.1") && e.contains("0 to 584.2") && e.contains("Set origin"), "{e}");
    // Negative space (GRBL homed to the far corner): the origin at -500, -1000 fits; at -100 it doesn't.
    let neg = MachineFrame { wco: Some([-500.0, -1000.0]), mpos: Some([-500.0, -1000.0]), ..pos };
    assert!(check_placement(job, bed, true, &neg).unwrap().is_empty());
    let neg_far = MachineFrame { wco: Some([-100.0, -1000.0]), mpos: Some([-100.0, -1000.0]), ..pos };
    assert!(check_placement(job, bed, true, &neg_far).unwrap_err().contains("-584.2 to 0"));
    // The head at 0: either way, so a warning when it fits only one; refused when it fits neither.
    let zero = MachineFrame { wco: Some([0.0, 0.0]), mpos: Some([0.0, 0.0]), ..pos };
    assert!(check_placement(job, bed, true, &zero).unwrap()[0].contains("can't tell which way"));
    let across = MachineFrame { wco: Some([-100.0, 0.0]), mpos: Some([0.0, 0.0]), ..pos };
    assert!(check_placement(job, bed, true, &across).is_err());
    // Without homing, machine coordinates mean nothing: size only.
    let unhomed = MachineFrame { homing: false, ..far };
    assert!(check_placement(job, bed, true, &unhomed).unwrap()[0].contains("only its size"));
    // The origin at the machine's: work coordinates within 0..bed.
    assert!(check_placement(job, bed, false, &MachineFrame::default()).unwrap().is_empty());
    assert!(check_placement(Rect::new(500.0, 0.0, 830.1, 180.1), bed, false, &MachineFrame::default()).unwrap_err().contains("off the bed"));
    assert!(check_placement(Rect::new(f64::NAN, 0.0, 1.0, 1.0), bed, true, &MachineFrame::default()).is_err());
}

#[test]
fn a_homed_machine_checks_the_job_in_machine_coordinates() {
    let mut s = connected(json!({"homing": true, "speedFactor": 1000, "timeScale": 100, "maxTravelMm": [584.2, 1219.2, 50]}));
    ex(&mut s, "laser.home", json!({}));
    wait(&mut s, 10.0, "homed", |v| v["state"] == "Idle" && v["log"].as_array().unwrap().iter().any(|l| l == "> $H"));
    // Jog to X 400 and set the origin there: a 330 mm sheet would run off the 584.2 mm travel.
    ex(&mut s, "laser.jog", json!({"x": 400, "y": 20}));
    wait(&mut s, 10.0, "the jog", |v| v["state"] == "Idle" && v["pos"]["x"] == 400.0);
    ex(&mut s, "laser.setOrigin", json!({}));
    wait(&mut s, 10.0, "the origin", |v| v["pos"]["x"] == 0.0 && v["mpos"]["x"] == 400.0);
    let sheet = json!({"gcode": "G21 G90 M5\nG0 X330.1 Y0\nG0 X330.1 Y180.1\nG0 X0 Y0\nM5\n"});
    let why = disabled(s.laser_execute_armed("laser.start", &sheet));
    assert!(why.contains("machine X 400 to 730.1") && why.contains("Set origin"), "{why}");
    assert!(disabled(s.execute("laser.frame", &sheet)).contains("machine X 400"));
    // Back to X 100: it fits, no warning.
    ex(&mut s, "laser.jog", json!({"x": -300, "y": 0}));
    wait(&mut s, 10.0, "the jog", |v| v["state"] == "Idle" && v["mpos"]["x"] == 100.0);
    ex(&mut s, "laser.setOrigin", json!({}));
    wait(&mut s, 10.0, "the origin", |v| v["pos"]["x"] == 0.0 && v["mpos"]["x"] == 100.0);
    let v = start(&mut s, sheet);
    assert!(v["started"]["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("can't tell")), "{v}");
    let v = wait(&mut s, 30.0, "done", |v| job_state(v) == "done");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
}

#[test]
fn gcode_bounds_reads_moves_and_refuses_what_it_cant_check() {
    let b = gcode_bounds("G21 G90\nG0 X10 Y10 ; travel\n(comment X999)\nG1 X60 Y10\nG2 X60 Y30 I0 J10\n").unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 10.0, 70.0, 30.0), "the arc as its whole circle (centre 60, 20)");
    let inch = gcode_bounds("G20 G90\nG0 X1 Y2\n").unwrap();
    assert_eq!((inch.x1, inch.y1), (25.4, 50.8));
    let rel = gcode_bounds("G90 G0 X10 Y10\nG91 G0 X5 Y-5\n").unwrap();
    assert_eq!((rel.x1, rel.y0), (15.0, 5.0));
    for (g, why) in [
        ("G53 G0 X0", "G53"),
        ("G28", "G28"),
        ("G92 X0", "G92"),
        ("G10 L20 P1 X0", "G10"),
        ("G55 G0 X1", "G55"),
        ("G91 G0 X5", "relative"),
        ("M5\n", "no moves"),
        ("G0 X2000000", "out of range"),
        ("G0 #1", "letter"),
        ("G0 X", "without a number"),
    ] {
        let e = gcode_bounds(g).unwrap_err();
        assert!(e.contains(why), "{g}: {e}");
    }
}

/// A document with a black and a white 40 pt square side by side.
fn two_squares(s: &mut Session) -> (u64, u64) {
    ex(s, "file.new", json!({"width": 300, "height": 300}));
    let mut sq = |x: f64, hex: &str| {
        let id = ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40}))["id"].as_u64().unwrap();
        ex(s, "paint.setFill", json!({"ids": [id], "color": hex}));
        id
    };
    (sq(0.0, "#000000"), sq(40.0, "#ffffff"))
}

fn decode_file(r: &Value) -> String {
    String::from_utf8(vectorcraft_format::base64_decode(r["files"][0]["data"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn job_files_are_checked_line_by_line_against_the_laser_allow_list() {
    for (line, why) in [
        ("$H", "`$` system commands"),
        ("$J=G91 X10 F1000", "`$` system commands"),
        ("$RST=*", "`$` system commands"),
        ("$32=0", "`$` system commands"),
        ("M3 S500", "M3 (constant power)"),
        ("G1 X1 Z-1", "Z moves"),
        ("G0 Z5", "Z moves"),
        ("G18", "arc plane"),
        ("G19 G2 X1 Y1 I1", "arc plane"),
        ("G20", "G20 (inches)"),
        ("G28", "can't be checked"),
        ("G30", "can't be checked"),
        ("G92 X0 Y0", "can't be checked"),
        ("G10 L20 P1 X0", "can't be checked"),
        ("G53 G0 X0", "can't be checked"),
        ("G55", "isn't allowed"),
        ("G38.2 X1", "isn't allowed"),
        ("G93", "isn't allowed"),
        ("M7", "M7 isn't allowed"),
        ("M0", "M0 isn't allowed"),
        ("M6 T1", "isn't allowed"),
        ("T1", "`T`"),
        ("G0 A5", "`A`"),
        ("%", "`%`"),
        ("G1 X1 S-1", "can't be negative"),
        ("G1 X1 F0", "more than 0"),
        ("G1 X1 P5", "G4 dwell"),
    ] {
        let e = check_line(&vectorcraft_grbl::clean_line(line).unwrap()).unwrap_err();
        assert!(e.contains(why), "{line}: {e}");
    }
    for line in [
        "G21",
        "G90",
        "G91",
        "G91.1",
        "G17",
        "G94",
        "G54",
        "M5",
        "M4 S100",
        "M8",
        "M9",
        "M2",
        "M30",
        "G4 P0.5",
        "N10 G1 X1 Y2 F600 S10",
        "G2 X1 Y1 I1 J0",
        "G3 X1 Y1 R5",
    ] {
        assert!(check_line(line).is_ok(), "{line}");
    }
    // The refusal names the line; nothing starts, even armed.
    let mut s = connected(fast());
    for (text, why) in [
        ("G21 G90\nG0 X10 Y10\n$H\nM5\n", "line 3"),
        ("G21 G90\nM3 S1000\nG1 X10 Y10 F600\n", "line 2 (`M3 S1000`)"),
        ("G21 G90\nG1 X10 Y10 Z-2 F600\n", "Z moves"),
        ("G20 G90\nG0 X1 Y1\n", "G20"),
        ("G21 G90\nG0 X1 Y1 ; fine\nG0 X1 Y1 !\n", "realtime"),
        ("; nothing\n(at all)\n", "no lines"),
    ] {
        let why2 = disabled(s.laser_execute_armed("laser.start", &json!({"gcode": text})));
        assert!(why2.contains(why), "{text:?}: {why2}");
        assert!(disabled(s.execute("laser.frame", &json!({"gcode": text}))).contains(why), "{text:?}");
    }
    assert_eq!(st(&mut s)["job"], Value::Null);
    assert_eq!(st(&mut s)["simStats"]["gcodeLines"], 0, "nothing was sent");
    // Picabia's own exports pass, every kind.
    let mut s = Session::new();
    let (a, b) = two_squares(&mut s);
    let design = decode_file(&ex(&mut s, "laser.gcode", json!({"ids": [a], "data": true})));
    check_gcode(&design).unwrap();
    let card = decode_file(&ex(&mut s, "modifier.stencilTestCard", json!({"format": "gcode", "data": true, "material": "film"})));
    check_gcode(&card).unwrap();
    let ladder = decode_file(&ex(&mut s, "modifier.stencilTestCard", json!({"kind": "powerLadder", "data": true})));
    check_gcode(&ladder).unwrap();
    let g = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [a, b], "props": {"widthMm": 80, "hoshi": false}}))["id"].as_u64().unwrap();
    ex(&mut s, "laser.profile", json!({"set": {"air": true}}));
    let stencil = decode_file(&ex(&mut s, "modifier.stencilGcode", json!({"id": g, "data": true})));
    assert!(stencil.contains("M8"), "air assist on");
    check_gcode(&stencil).unwrap();
}

#[test]
fn gcode_bounds_applies_a_lines_modal_words_before_its_coordinates() {
    let b = gcode_bounds("X1 Y2 G20 G0\n").unwrap();
    assert_eq!((b.x1, b.y1), (25.4, 50.8), "G20 later on the line still applies");
    let b = gcode_bounds("G90 G0 X10 Y10\nX5 Y-5 G91\n").unwrap();
    assert_eq!((b.x1, b.y0), (15.0, 5.0), "G91 later on the line still applies");
    let b = gcode_bounds("G90 G0 X0 Y0\nX20 Y0 I10 J0 G2\n").unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, -10.0, 20.0, 10.0), "the arc's circle, G2 after its words");
    let b = gcode_bounds("G90 G0 X0 Y0\nG3 X10 Y0 R5\n").unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (-10.0, -10.0, 10.0, 10.0), "R arcs: within 2R of the start");
}

#[test]
fn any_objects_and_stencils_can_be_cut() {
    let mut s = connected(fast());
    let (a, b) = two_squares(&mut s);
    // An object (the selection or `ids`), at a point a unit.
    let v = start(&mut s, json!({"ids": [a]}));
    let mm = 40.0 * 25.4 / 72.0;
    let bounds = &v["started"]["boundsMm"];
    assert!((bounds["x1"].as_f64().unwrap() - mm).abs() < 0.01 && (bounds["y1"].as_f64().unwrap() - mm).abs() < 0.01, "{v}");
    let v = wait(&mut s, 20.0, "done", |v| job_state(v) == "done");
    assert_eq!(v["job"]["source"], "design");
    assert_eq!(v["job"]["layer"], a);
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    assert!(v["simStats"]["burnMm"].as_f64().unwrap() > 4.0 * mm, "{v}");
    ex(&mut s, "laser.frame", json!({"ids": [a], "widthMm": 100}));
    let v = wait(&mut s, 20.0, "the frame", |v| v["state"] == "Idle" && v["pos"]["x"] == 0.0 && v["simStats"]["movedMm"].as_f64().unwrap() > 400.0);
    assert!(v["pos"]["y"] == 0.0, "{v}");
    // A stencil object's sheet.
    let g = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [a, b], "props": {"widthMm": 80, "hoshi": false}}))["id"].as_u64().unwrap();
    let v = start(&mut s, json!({"id": g}));
    assert!(v["started"]["lines"].as_u64().unwrap() > 10, "{v}");
    let v = wait(&mut s, 30.0, "the stencil", |v| job_state(v) == "done");
    assert_eq!((v["job"]["source"].as_str(), v["job"]["sheet"].as_str()), (Some("stencil"), Some("1 of 1")));
    assert_eq!(v["job"]["layer"], g);
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    // Junk for objects.
    let e = s.laser_execute_armed("laser.start", &json!({"id": g, "sheet": 9})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("sheet")), "{e}");
    assert!(s.laser_execute_armed("laser.start", &json!({"ids": [a], "sheet": 1})).is_err());
    assert!(s.laser_execute_armed("laser.start", &json!({"gcode": squares(1), "ids": [a]})).is_err());
    ex(&mut s, "select.set", json!({"ids": []}));
    assert!(s.laser_execute_armed("laser.start", &json!({})).is_err(), "nothing selected");
}

#[test]
fn the_test_card_uses_the_materials_saved_settings() {
    let mut s = connected(fast());
    let quick = start(&mut s, json!({"testCard": "card"}))["started"]["estimatedSec"].as_f64().unwrap();
    wait(&mut s, 30.0, "the card", |v| job_state(v) == "done");
    ex(&mut s, "laser.profile", json!({"set": {"materials": {"paper": {"cut": {"speedMmMin": 30, "passes": 1}}}}}));
    let slow = start(&mut s, json!({"testCard": "card"}))["started"]["estimatedSec"].as_f64().unwrap();
    assert!(slow > quick * 2.0, "the saved slower cut is used: {quick} s then {slow} s");
    ex(&mut s, "laser.stop", json!({}));
}

#[test]
fn a_start_seen_after_its_deadline_never_runs() {
    let mut ctrl = vectorcraft_grbl::Controller::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let late = super::native::Msg::Start {
        name: "late".into(),
        lines: vec!["G0 X1".into()],
        deadline: Instant::now() - Duration::from_millis(1),
        reply: tx,
    };
    let (_, r) = super::native::handle(&mut ctrl, late).unwrap();
    assert!(r.unwrap_err().contains("too late"));
    assert!(ctrl.job().is_none(), "dropped, never started");
    drop(rx);
}

#[test]
fn a_stop_handle_stops_from_another_thread() {
    let mut s = connected(slow());
    assert!(Session::new().laser_stop_handle().is_none(), "nothing to stop when not connected");
    let h = s.laser_stop_handle().unwrap();
    start(&mut s, job(20));
    wait(&mut s, 10.0, "burning", |v| v["simStats"]["burnMm"].as_f64().unwrap() > 1.0);
    let before = h.stops_done();
    std::thread::spawn(move || h.stop()).join().unwrap();
    let v = wait(&mut s, 5.0, "the stop", |v| job_state(v) == "stopped" && v["stopping"] == false && v["state"] == "Idle");
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1, "{v}");
    assert!(s.laser_stop_handle().unwrap().stops_done() > before);
    // laser.stop and laser.pause report the worker's confirmation.
    start(&mut s, job(20));
    let v = ex(&mut s, "laser.pause", json!({}));
    assert_eq!(job_state(&v), "paused");
    let v = ex(&mut s, "laser.stop", json!({}));
    assert_eq!(job_state(&v), "stopped");
}

#[test]
fn a_write_timeout_mid_job_holds_and_stops_but_keeps_the_link() {
    let mut s = connected(json!({"speedFactor": 1000, "timeScale": 100, "writeTimeoutAtLine": 20}));
    start(&mut s, job(20));
    let v = wait(&mut s, 20.0, "the stopped job", |v| {
        job_state(v) == "stopped" && v["stopping"] == false && (v["state"] == "Alarm" || v["state"] == "Idle")
    });
    assert_eq!(v["connected"], true, "a timeout isn't a pulled cable: {v}");
    assert!(v["lastError"].as_str().unwrap().contains("timed out"), "{v}");
    assert!(v["simStats"]["resets"].as_u64().unwrap() >= 1, "the hold and reset went out: {v}");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
    // A reset while the head still moved locks GRBL (alarm 3) until it is unlocked or homed.
    if v["state"] == "Alarm" {
        assert_eq!(v["alarm"]["code"], 3, "{v}");
        ex(&mut s, "laser.unlock", json!({}));
        wait(&mut s, 5.0, "unlocked", |v| v["state"] == "Idle");
    }
    // The link still works.
    start(&mut s, job(1));
    wait(&mut s, 20.0, "the next job", |v| job_state(v) == "done");
}

#[test]
fn a_worker_panic_closes_the_link_and_fails_the_job() {
    let mut s = connected(json!({"speedFactor": 1000, "timeScale": 100, "panicAtLine": 15}));
    start(&mut s, job(20));
    let v = wait(&mut s, 20.0, "the failed link", |v| v["connected"] == false);
    assert_eq!(job_state(&v), "failed", "{v}");
    assert!(v["lastError"].as_str().unwrap().contains("internal error"), "{v}");
    assert!(disabled(s.execute("laser.home", &json!({}))).contains("isn't connected"));
    ex(&mut s, "laser.stop", json!({}));
    // Connecting again works.
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": fast()}));
    wait(&mut s, 10.0, "ready again", |v| v["phase"] == "ready");
}
