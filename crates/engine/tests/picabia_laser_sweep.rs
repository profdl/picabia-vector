//! PICABIA: the every-command junk sweep for the `laser.*` commands, on each fixture, with the
//! link closed and connected to the simulated GRBL machine: each call returns Ok or Err, never
//! panics, never arms the session, never leaves the simulator for a real port, and never starts a
//! job (nothing here can arm). These sessions never enable real ports
//! (`Session::laser_enable_hardware`, the desktop app's), so no device is listed or opened.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::{EngineError, Session, command_specs};
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::Fixture;
use vectorcraft_testkit::strategies::junk_params;

fn status(s: &mut Session) -> Value {
    s.execute("laser.status", &json!({})).unwrap()
}

/// `s` connected to a fast simulator, ready.
fn connect(s: &mut Session) {
    s.execute("laser.connect", &json!({"port": "sim", "sim": {"speedFactor": 1000, "timeScale": 100}})).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while status(s)["phase"] != "ready" {
        assert!(Instant::now() < end, "the simulator never became ready: {}", status(s));
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn laser_commands_with_junk_params() {
    let path = vectorcraft_testkit::temp_dir("picabia-laser-sweep").join("out.gcode").to_string_lossy().to_string();
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("laser.")).collect();
    assert!(ids.len() >= 14, "{ids:?}");
    let mut failures = vec![];
    let mut calls = 0usize;
    for connected in [false, true] {
        for fx in Fixture::ALL {
            let mut s = fx.session();
            if connected {
                connect(&mut s);
            }
            for id in &ids {
                let spec = command_specs().iter().find(|c| c.id == *id).unwrap();
                let mut cases = junk_params(spec.params, &path);
                cases.push(json!({}));
                cases.push(json!({"port": "/nonexistent/picabia-sweep-port"}));
                cases.push(json!({"gcode": "G21 G90\nG0 X10 Y10\nM4 S100\nG1 X20 Y10 F600\nM5\n"}));
                for p in &cases {
                    calls += 1;
                    let r = catch_quiet(|| s.execute(id, p));
                    let fail = match r {
                        Err(msg) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                        Ok(Err(EngineError::Internal { msg, .. })) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                        Ok(_) => None,
                    };
                    failures.extend(fail);
                }
                let v = status(&mut s);
                if v["armed"] != false {
                    failures.push(format!("{id} [{fx:?}]: armed the session"));
                }
                if !(v["port"].is_null() || v["port"] == "sim") || v["hardware"] != false {
                    failures.push(format!("{id} [{fx:?}]: reached a real port: {v}"));
                }
                if !v["job"].is_null() && v["job"]["source"] != "frame" {
                    failures.push(format!("{id} [{fx:?}]: started a job without the arm: {v}"));
                }
            }
            let _ = s.execute("laser.stop", &json!({}));
        }
    }
    assert!(calls > 500, "only {calls} calls");
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} failures (of {calls} calls):\n{}",
        failures.len(),
        failures.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// Without the desktop app, the ports are the simulator's alone, and a real port is refused before
/// anything is opened.
#[test]
fn only_the_simulator_outside_the_desktop_app() {
    let mut s = Session::new();
    let ports = s.execute("laser.ports", &json!({})).unwrap();
    assert_eq!(ports["ports"], json!([{"path": "sim", "name": "Simulated GRBL", "usb": false, "sim": true}]));
    for port in ["/nonexistent/picabia-sweep-port", "COM3", "junk"] {
        let r = s.execute("laser.connect", &json!({"port": port}));
        assert!(matches!(&r, Err(EngineError::Disabled(_, why)) if why.contains("only in the desktop app")), "{port}: {r:?}");
    }
    assert_eq!(status(&mut s)["phase"], "closed");
}

/// The control channel, MCP and the CLI run commands through `execute` (and batches): a start, a
/// beam frame and a resume are refused there, and stop works.
#[test]
fn execute_never_arms_and_stop_always_works() {
    // A batch needs a document.
    let mut s = Fixture::Empty.session();
    connect(&mut s);
    let gcode = "G21 G90\nG0 X10 Y10\nM4 S100\nG1 X20 Y10 F600\nM5\n";
    for (id, p) in [
        ("laser.start", json!({"gcode": gcode})),
        ("laser.frame", json!({"gcode": gcode, "beam": true})),
        ("laser.start", json!({"testCard": "powerLadder"})),
    ] {
        let r = s.execute(id, &p);
        assert!(matches!(&r, Err(EngineError::Disabled(_, why)) if why.contains("isn't armed")), "{id}: {r:?}");
        let r = s.execute("command.batch", &json!({"commands": [{"command": id, "params": p}]}));
        assert!(r.is_err(), "{id} in a batch: {r:?}");
    }
    let v = status(&mut s);
    assert!(v["job"].is_null(), "{v}");
    assert_eq!(v["simStats"]["burnMm"], 0.0);
    let v = s.execute("laser.stop", &json!({})).unwrap();
    assert_eq!(v["connected"], true);
    s.execute("command.batch", &json!({"commands": [{"command": "laser.stop", "params": {}}]})).unwrap();
}
