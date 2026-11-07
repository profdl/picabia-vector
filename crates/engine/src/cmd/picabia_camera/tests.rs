use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::Session;

/// A fresh empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("picabia-camera-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Wait (at most 5 s) until the status passes `f`.
fn wait_for(s: &mut Session, f: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let st = s.execute("camera.status", &json!({})).unwrap();
        if f(&st) || Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn streaming(s: &mut Session, sim: Value) -> Value {
    s.execute("camera.connect", &json!({"ip": "sim", "sim": sim})).unwrap();
    let st = wait_for(s, |st| st["phase"] == "streaming" && st["frames"].as_u64().unwrap_or(0) > 2);
    assert_eq!(st["phase"], "streaming", "{st}");
    st
}

#[test]
fn closed_until_connected() {
    let mut s = Session::new();
    let st = s.execute("camera.status", &json!({})).unwrap();
    assert_eq!(st["phase"], "closed");
    assert_eq!(st["connected"], false);
    assert_eq!(st["folder"], Value::Null);
    assert_eq!(st["prefix"], "frame");
    assert!(s.execute("camera.capture", &json!({})).is_err());
    assert!(s.execute("camera.stop", &json!({})).is_err());
    assert!(s.execute("camera.frame", &json!({})).is_err());
}

#[test]
fn real_cameras_need_the_network_switch() {
    let mut s = Session::new();
    let e = s.execute("camera.connect", &json!({"ip": "192.168.54.1"})).unwrap_err().to_string();
    assert!(e.contains("desktop app"), "{e}");
    let e = s.execute("camera.connect", &json!({})).unwrap_err().to_string();
    assert!(e.contains("desktop app"), "the default address is a real camera too: {e}");
    assert!(s.execute("camera.find", &json!({})).is_err());
    assert!(s.execute("camera.connect", &json!({"ip": "not an ip"})).is_err());
    assert!(s.execute("camera.connect", &json!({"ip": 5})).is_err());
    assert!(s.execute("camera.connect", &json!({"ip": "192.168.54.1", "sim": {}})).is_err());
    assert_eq!(s.execute("camera.status", &json!({})).unwrap()["phase"], "closed", "nothing opened");
}

#[test]
fn streams_from_the_simulator() {
    let mut s = Session::new();
    let st = streaming(&mut s, json!({"fps": 30}));
    assert_eq!(st["sim"], true);
    assert_eq!(st["ip"], "sim");
    assert_eq!(st["camera"]["batt"], "3/3", "the camera's getstate");
    let f = s.camera_frame().unwrap();
    assert_eq!((f.width, f.height), (640, 480));
    assert_eq!(f.rgba.len(), 640 * 480 * 4);
    let fr = s.execute("camera.frame", &json!({})).unwrap();
    assert!(fr["bytes"].as_u64().unwrap() > 100);
    assert!(fr["jpegBase64"].as_str().unwrap().starts_with("/9j/"), "a JPEG's base64 starts with FF D8 FF");
    s.execute("camera.disconnect", &json!({})).unwrap();
    assert_eq!(s.execute("camera.status", &json!({})).unwrap()["phase"], "closed");
}

#[test]
fn handshake_when_the_camera_wants_one() {
    let mut s = Session::new();
    let st = streaming(&mut s, json!({"requireHandshake": true}));
    assert_eq!(st["handshaken"], true);
    assert!(CameraPrefs::read(&s).device_id.unwrap().starts_with("4D454930-"), "the id is kept");
}

#[test]
fn a_new_computer_waits_for_the_camera_to_accept_it() {
    let mut s = Session::new();
    s.execute("camera.connect", &json!({"ip": "sim", "sim": {"requireHandshake": true, "acceptAfterMs": 1500}})).unwrap();
    let st = wait_for(&mut s, |st| st["awaitingAccept"] == true);
    assert_eq!(st["phase"], "connecting", "{st}");
    assert!(st["hint"].as_str().unwrap().contains("accept"), "{st}");
    let st = wait_for(&mut s, |st| st["phase"] == "streaming");
    assert_eq!(st["phase"], "streaming", "{st}");
    assert_eq!(st["awaitingAccept"], false);
    assert_eq!(st["handshaken"], true);
}

#[test]
fn captures_numbered_frames_into_the_folder() {
    let mut s = Session::new();
    let dir = folder("numbered");
    std::fs::write(dir.join("frame_0007.jpg"), b"old").unwrap();
    std::fs::write(dir.join("frame_0099.png"), b"not a jpeg").unwrap();
    std::fs::write(dir.join("other_0050.jpg"), b"another prefix").unwrap();
    streaming(&mut s, json!({}));
    let e = s.execute("camera.capture", &json!({})).unwrap_err().to_string();
    assert!(e.contains("capture folder"), "{e}");
    let st = s.execute("camera.settings", &json!({"folder": dir.to_string_lossy()})).unwrap();
    assert_eq!(st["nextFile"], "frame_0008.jpg");
    let r = s.execute("camera.capture", &json!({})).unwrap();
    assert_eq!(r["saved"], true, "{r}");
    assert_eq!(r["number"], 8);
    assert_eq!(r["file"], "frame_0008.jpg");
    let bytes = std::fs::read(dir.join("frame_0008.jpg")).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xD8]) && bytes.ends_with(&[0xFF, 0xD9]));
    let r = s.execute("camera.capture", &json!({})).unwrap();
    assert_eq!(r["file"], "frame_0009.jpg");
    assert_eq!(std::fs::read(dir.join("frame_0007.jpg")).unwrap(), b"old", "nothing replaced");
    // A new prefix starts its own sequence.
    s.execute("camera.settings", &json!({"prefix": "shot-A"})).unwrap();
    assert_eq!(s.execute("camera.capture", &json!({})).unwrap()["file"], "shot-A_0001.jpg");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_refused_capture_saves_nothing() {
    let mut s = Session::new();
    let dir = folder("refused");
    s.execute("camera.settings", &json!({"folder": dir.to_string_lossy()})).unwrap();
    streaming(&mut s, json!({"refuseCapture": true}));
    let e = s.execute("camera.capture", &json!({})).unwrap_err().to_string();
    assert!(e.contains("err_busy") && e.contains("nothing was saved"), "{e}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stopped_live_view_still_fires_but_saves_no_frame() {
    let mut s = Session::new();
    let dir = folder("stopped");
    s.execute("camera.settings", &json!({"folder": dir.to_string_lossy()})).unwrap();
    streaming(&mut s, json!({}));
    s.execute("camera.stop", &json!({})).unwrap();
    let st = wait_for(&mut s, |st| st["phase"] == "connected");
    assert_eq!(st["liveView"], false);
    let r = s.execute("camera.capture", &json!({})).unwrap();
    assert_eq!(r["shutter"], true);
    assert_eq!(r["saved"], false);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    s.execute("camera.start", &json!({})).unwrap();
    assert_eq!(wait_for(&mut s, |st| st["phase"] == "streaming")["phase"], "streaming");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_offline_camera_is_lost_with_a_hint() {
    let mut s = Session::new();
    s.execute("camera.connect", &json!({"ip": "sim", "sim": {"offline": true}})).unwrap();
    let st = wait_for(&mut s, |st| st["phase"] == "lost");
    assert_eq!(st["phase"], "lost", "{st}");
    assert!(st["hint"].as_str().unwrap().contains("No answer from sim"));
    assert!(st["lastError"].is_string());
    assert!(s.execute("camera.capture", &json!({})).is_err());
}

#[test]
fn settings_are_checked_and_remembered() {
    let mut s = Session::new();
    let dir = folder("settings");
    assert!(s.execute("camera.settings", &json!({"folder": dir.join("missing").to_string_lossy()})).is_err());
    let file = dir.join("a.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(s.execute("camera.settings", &json!({"folder": file.to_string_lossy()})).is_err(), "a file isn't a folder");
    assert!(s.execute("camera.settings", &json!({"folder": 3})).is_err());
    assert!(s.execute("camera.settings", &json!({"prefix": "../up"})).is_err());
    assert!(s.execute("camera.settings", &json!({"prefix": "x".repeat(41)})).is_err());
    assert!(s.execute("camera.settings", &json!({"place": "yes"})).is_err());
    let st = s.execute("camera.settings", &json!({"folder": dir.to_string_lossy(), "prefix": " take 2 ", "place": true})).unwrap();
    assert_eq!(st["prefix"], "take 2");
    assert_eq!(st["place"], true);
    assert_eq!(st["nextFile"], "take 2_0001.jpg");
    let p = CameraPrefs::read(&s);
    assert_eq!(p.folder.as_deref(), Some(dir.to_string_lossy().as_ref()));
    assert_eq!(s.prefs.picabia_camera["prefix"], "take 2", "kept in the preferences");
    // An empty prefix goes back to the default.
    assert_eq!(s.execute("camera.settings", &json!({"prefix": ""})).unwrap()["prefix"], "frame");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn numbers_read_from_names() {
    assert_eq!(number_in("frame_0012.jpg", "frame_"), Some(12));
    assert_eq!(number_in("frame_12.JPEG", "frame_"), Some(12));
    assert_eq!(number_in("frame_0012.png", "frame_"), None);
    assert_eq!(number_in("frame_.jpg", "frame_"), None);
    assert_eq!(number_in("frame_12a.jpg", "frame_"), None);
    assert_eq!(number_in("frame_1234567.jpg", "frame_"), None, "at most 6 digits");
    assert_eq!(number_in("frames_0001.jpg", "frame_"), None);
    assert_eq!(file_name("frame", 3), "frame_0003.jpg");
    assert_eq!(file_name("frame", 123_456), "frame_123456.jpg");
}

#[test]
fn simulator_options_are_checked() {
    let mut s = Session::new();
    for bad in [json!({"fps": 0}), json!({"fps": 61}), json!({"offline": 1}), json!({"streamTimeoutMs": -1}), json!([])] {
        assert!(s.execute("camera.connect", &json!({"ip": "sim", "sim": bad})).is_err(), "{bad}");
    }
}

#[test]
fn device_ids_look_like_uuids() {
    let id = new_device_id();
    assert_eq!(id.len(), 36);
    assert_eq!(id.matches('-').count(), 4);
}
