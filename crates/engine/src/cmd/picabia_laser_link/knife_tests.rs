//! PICABIA (PIC35): the drag knife as the machine profile's head, against the simulated GRBL
//! machine only: jobs wait for Set origin to zero Z, run with Z and never the beam, frames lift
//! first, laser files are refused, and the knife's allow-list.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{KNIFE_Z_UNSET, check_gcode_for, check_line, check_line_for};
use crate::{EngineError, Session};
use vectorcraft_stencil::Head;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn wait(s: &mut Session, secs: f64, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    loop {
        let v = ex(s, "laser.status", json!({}));
        if done(&v) {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: {v:#}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Connected to a fast simulator with the knife as the profile's head.
fn knife_session() -> Session {
    let mut s = Session::new();
    ex(&mut s, "laser.profile", json!({"set": {"head": "knife"}}));
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": {"speedFactor": 1000, "timeScale": 100}}));
    wait(&mut s, 10.0, "ready", |v| v["phase"] == "ready" && v["state"] == "Idle");
    s
}

fn start(s: &mut Session, p: Value) -> crate::Result<Value> {
    s.laser_execute_armed("laser.start", &p)
}

fn disabled(r: crate::Result<Value>) -> String {
    match r {
        Err(EngineError::Disabled(_, why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn job_state(v: &Value) -> &str {
    v["job"]["state"].as_str().unwrap_or("")
}

/// A 40 pt square.
fn square(s: &mut Session) -> u64 {
    ex(s, "file.new", json!({"width": 300, "height": 300}));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40}))["id"].as_u64().unwrap()
}

#[test]
fn knife_jobs_wait_for_set_origin_then_cut_on_z_without_the_beam() {
    let mut s = knife_session();
    let id = square(&mut s);
    let v = ex(&mut s, "laser.status", json!({}));
    assert_eq!((v["profile"]["head"].as_str(), v["knifeZSet"].as_bool(), v["canStart"].as_bool()), (Some("knife"), Some(false), Some(false)));
    assert_eq!(v["startBlockedBy"], json!([KNIFE_Z_UNSET]), "{v}");
    assert_eq!(disabled(start(&mut s, json!({"ids": [id]}))), KNIFE_Z_UNSET);
    assert_eq!(disabled(s.laser_execute_person("laser.frame", &json!({"ids": [id]}))), KNIFE_Z_UNSET);
    // Lower the blade onto the sheet, then Set origin: it zeroes Z too.
    let j = ex(&mut s, "laser.jog", json!({"dz": -2.5, "feed": 300}));
    assert_eq!(j["sent"], json!(["$J=G91 G21 Z-2.5 F300"]));
    wait(&mut s, 10.0, "the jog", |v| v["state"] == "Idle" && v["pos"]["z"].as_f64().is_some_and(|z| (z + 2.5).abs() < 1e-6));
    let o = ex(&mut s, "laser.setOrigin", json!({}));
    assert_eq!(o["sent"], json!(["G10 L20 P1 X0 Y0 Z0"]));
    let v = wait(&mut s, 10.0, "Z 0", |v| v["pos"]["z"].as_f64().is_some_and(|z| z.abs() < 1e-6));
    assert_eq!((v["knifeZSet"].as_bool(), v["canStart"].as_bool()), (Some(true), Some(true)), "{v}");
    // The square: knife G-code, cut with the blade down and never the beam.
    let r = start(&mut s, json!({"ids": [id]})).unwrap();
    assert!(r["started"]["lines"].as_u64().unwrap() > 8, "{r}");
    let v = wait(&mut s, 30.0, "the cut", |v| job_state(v) == "done" && v["state"] == "Idle");
    let sim = &v["simStats"];
    assert_eq!((sim["burnMm"].as_f64(), sim["beamOnTravelMm"].as_f64()), (Some(0.0), Some(0.0)), "never the beam: {sim}");
    let mm = 40.0 * 25.4 / 72.0;
    assert!(sim["movedMm"].as_f64().unwrap() > 4.0 * mm, "{sim}");
    assert!((v["pos"]["z"].as_f64().unwrap() - 5.0).abs() < 1e-6, "parked at the travel height: {v}");
    assert!(v["job"]["name"].as_str().unwrap().ends_with(" knife"), "named for the tool: {v}");
    // A frame lifts to the travel height first; never with the beam.
    let f = s.laser_execute_person("laser.frame", &json!({"ids": [id]})).unwrap();
    assert_eq!(f["sent"][0], "M5");
    assert!(f["sent"][1].as_str().unwrap().starts_with("$J=G90 G21 Z5 "), "{f}");
    wait(&mut s, 20.0, "the frame", |v| v["state"] == "Idle" && v["pos"]["x"] == 0.0);
    assert!(disabled(s.laser_execute_armed("laser.frame", &json!({"ids": [id], "beam": true}))).contains("no beam"));
    // A new connection forgets Z: GRBL may have kept another work offset.
    ex(&mut s, "laser.disconnect", json!({}));
    ex(&mut s, "laser.connect", json!({"port": "sim", "sim": {"speedFactor": 1000, "timeScale": 100}}));
    let v = wait(&mut s, 10.0, "ready again", |v| v["phase"] == "ready" && v["state"] == "Idle");
    assert_eq!(v["knifeZSet"], false);
    assert_eq!(disabled(start(&mut s, json!({"ids": [id]}))), KNIFE_Z_UNSET);
}

#[test]
fn a_knife_refuses_laser_files_the_power_ladder_and_a_head_param() {
    let mut s = knife_session();
    ex(&mut s, "laser.setOrigin", json!({}));
    let laser = "G21 G90 M5\nG0 X10 Y10\nM4 S300\nG1 X20 Y10 F1200\nM5\n";
    assert!(disabled(start(&mut s, json!({"gcode": laser}))).contains("M4 turns the spindle"));
    let knife = "G21 G90 M5\nG0 Z5\nG0 X10 Y10\nG1 Z-0.5 F300\nG1 X20 Y10 F1000\nG0 Z5\nG0 X0 Y0\nM5\n";
    start(&mut s, json!({"gcode": knife})).unwrap();
    let v = wait(&mut s, 20.0, "the knife file", |v| job_state(v) == "done");
    assert_eq!(v["simStats"]["burnMm"], 0.0);
    assert!(disabled(start(&mut s, json!({"testCard": "powerLadder"}))).contains("power ladder is for the laser"));
    let e = start(&mut s, json!({"testCard": "card", "head": "laser"})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("`head`")), "the head is the profile's, never a job's: {e}");
    // The test card cuts with the knife (its label isn't engraved).
    let r = start(&mut s, json!({"testCard": "card"})).unwrap();
    assert!(r["started"]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("doesn't engrave")), "{r}");
    let v = wait(&mut s, 30.0, "the card", |v| job_state(v) == "done");
    assert_eq!(v["simStats"]["beamOnTravelMm"], 0.0);
}

#[test]
fn the_knife_allow_list_takes_z_and_refuses_the_spindle() {
    for line in ["G0Z5", "G1Z-0.5F300", "G2X1Y1I0.5J0F900", "G3X1Y1I0J0.5", "M5", "S0", "G0X1Y2Z3"] {
        assert!(check_line_for(line, Head::Knife).is_ok(), "{line}");
    }
    for (line, why) in [
        ("M3S500", "M3 turns the spindle"),
        ("M4", "M4 turns the spindle"),
        ("S300", "laser's power"),
        ("$H", "`$` system"),
        ("G28", "can't be checked"),
    ] {
        let e = check_line_for(line, Head::Knife).unwrap_err();
        assert!(e.contains(why), "{line}: {e}");
    }
    // The laser's list is unchanged: still no Z.
    assert!(check_line("G0Z5").unwrap_err().contains("Z moves"));
    assert_eq!(check_line_for("G0Z5", Head::Laser), check_line("G0Z5"));
    let e = check_gcode_for("G21\nG0 Z5\nM4 S10\n", Head::Knife).unwrap_err();
    assert!(e.starts_with("line 3"), "{e}");
}

#[test]
fn exports_follow_the_profiles_head_with_no_kerf() {
    let mut s = Session::new();
    let id = square(&mut s);
    let laser = ex(&mut s, "laser.gcode", json!({"ids": [id], "data": true}));
    ex(&mut s, "laser.profile", json!({"set": {"head": "knife", "knife": {"cutZMm": -0.7, "offsetMm": 0.3}}}));
    let p = ex(&mut s, "laser.profile", json!({}));
    assert_eq!((p["profile"]["head"].as_str(), p["profile"]["knife"]["cutZMm"].as_f64()), (Some("knife"), Some(-0.7)));
    let knife = ex(&mut s, "laser.gcode", json!({"ids": [id], "data": true}));
    let text = |r: &Value| String::from_utf8(vectorcraft_format::base64_decode(r["files"][0]["data"].as_str().unwrap()).unwrap()).unwrap();
    let (lt, kt) = (text(&laser), text(&knife));
    assert!(lt.contains("M4 S") && !lt.contains('Z'), "laser: {lt}");
    assert!(kt.contains("G1 Z-0.7 F") && !kt.contains("M4") && (kt.contains("G2 ") || kt.contains("G3 ")), "knife: {kt}");
    assert_eq!(knife["gcode"]["head"], "knife");
    // A stencil's sheet: the knife takes no kerf, so the cut-outs aren't grown.
    let st = ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [id], "props": {"widthMm": 80, "hoshi": false}}))["id"].as_u64().unwrap();
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": st, "dryRun": true}));
    assert_eq!(r["params"]["kerfMm"], 0.0, "{r}");
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": st, "dryRun": true, "kerfMm": 0.05}));
    assert_eq!(r["params"]["kerfMm"], 0.05, "a given kerf still wins");
}
