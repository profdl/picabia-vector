//! PICABIA: laser export of stencil objects (SVG and G-code), the test card and power ladder,
//! `laser.gcode` for any objects, and the machine profile. Text only: nothing here talks to a
//! machine.

use serde_json::{Value, json};

use super::laser_gcode::MachineProfile;
use super::stencil_tests::{BLACK, WHITE, rect, two_tone};
use super::tests::{ex, id_of, session};
use crate::{EngineError, Prefs, Session};

fn tmp(name: &str) -> std::path::PathBuf {
    vectorcraft_testkit::temp_dir(&format!("picabia-laser-{name}"))
}

fn decode(f: &Value) -> String {
    String::from_utf8(vectorcraft_format::base64_decode(f["data"].as_str().unwrap()).unwrap()).unwrap()
}

fn close(a: &Value, b: f64) -> bool {
    a.as_f64().is_some_and(|a| (a - b).abs() < 1e-6)
}

fn pref(s: &Session, key: &str) -> Value {
    match key {
        "laser" => s.prefs.picabia_laser.clone(),
        _ => s.prefs.picabia_machine.clone(),
    }
}

/// Nothing Picabia remembered.
fn nothing_remembered(s: &Session) -> bool {
    s.prefs.picabia_laser.is_null() && s.prefs.picabia_machine.is_null()
}

/// The X and Y of a motion line (`G0`/`G1`).
fn xy(line: &str) -> Option<(f64, f64)> {
    if !(line.starts_with("G0 ") || line.starts_with("G1 ")) {
        return None;
    }
    let word = |c: char| line.split_whitespace().find_map(|w| w.strip_prefix(c)).and_then(|v| v.parse::<f64>().ok());
    Some((word('X')?, word('Y')?))
}

/// The G-code safety rules the writer keeps: GRBL laser mode with dynamic power (`M4`, never
/// `M3`), mm and absolute, beam off before the first motion and at the end, the first and last
/// motions travels.
fn assert_safe(text: &str) {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.starts_with(';') && !l.is_empty()).collect();
    assert!(!lines.iter().any(|l| l.starts_with("M3")), "never constant power: {text}");
    assert!(lines.contains(&"G21") && lines.contains(&"G90"), "mm, absolute: {text}");
    let first_motion = lines.iter().position(|l| xy(l).is_some()).unwrap();
    assert!(lines[..first_motion].contains(&"M5"), "beam off before the first motion: {lines:?}");
    assert!(lines[first_motion].starts_with("G0"), "the first motion is a travel: {}", lines[first_motion]);
    let last_motion = lines.iter().rposition(|l| xy(l).is_some()).unwrap();
    assert!(lines[last_motion].starts_with("G0"), "the last motion (the park) is a travel: {}", lines[last_motion]);
    assert_eq!(lines.last(), Some(&"M5"), "beam off at the end");
    if let Some(last_burn) = lines.iter().rposition(|l| l.starts_with("G1")) {
        assert!(lines[last_burn..last_motion].contains(&"M5"), "beam off before the park");
    }
    assert!(lines.iter().any(|l| l.starts_with("M4")), "dynamic power: {text}");
}

#[test]
fn export_writes_a_true_size_sheet_with_its_cut_layer() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let dir = tmp("one");
    let path = dir.join("rose.svg");
    let r = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "path": path.to_string_lossy()}));
    assert_eq!(r["sheets"], 1, "{r}");
    let f = &r["files"][0];
    assert_eq!(f["path"], json!(path.to_string_lossy()));
    assert_eq!(f["name"], "rose.svg");
    assert_eq!(f["sheet"], "1 of 1");
    // The sheet plus the kerf (half each side) of paper's 0.10 mm.
    assert!(close(&f["wMm"], 330.1) && close(&f["hMm"], 180.1), "{f}");
    assert!(f["contours"].as_u64().unwrap() >= 6, "the white square, four registration holes and the outline: {f}");
    assert_eq!(f["loosePieces"], 0);
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(svg.contains("width=\"330.1mm\" height=\"180.1mm\""), "{svg}");
    assert!(svg.contains("stroke=\"#ff0000\""), "{svg}");
    assert_eq!(r["params"]["kerfMm"], 0.1);
    assert_eq!(r["params"]["cutColour"], "#ff0000");
    // Remembered for next time.
    assert_eq!(pref(&s, "laser"), r["params"]);
}

#[test]
fn data_mode_returns_the_files_as_base64() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let r = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "data": true, "cutColour": "#00ff00", "label": false}));
    let f = &r["files"][0];
    assert_eq!(f["name"], "Stencil laser.svg");
    assert!(f.get("path").is_none());
    let svg = decode(f);
    assert!(svg.starts_with("<?xml") && svg.contains("width=\"330.1mm\""), "{svg}");
    assert!(svg.contains("stroke=\"#00ff00\""), "{svg}");
    assert_eq!(r["params"]["label"], false);
}

#[test]
fn a_dry_run_describes_the_sheets_and_remembers_nothing() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    for cmd in ["modifier.stencilExport", "modifier.stencilTestCard", "modifier.stencilGcode"] {
        let r = ex(&mut s, cmd, json!({"id": g.0, "dryRun": true, "data": true, "kerfMm": 0.3}));
        let f = &r["files"][0];
        assert!(f["wMm"].as_f64().unwrap() > 0.0 && f["name"].is_string(), "{cmd}: {r}");
        assert!(f.get("data").is_none() && f.get("path").is_none(), "{cmd}: {r}");
        assert_eq!(r["params"]["kerfMm"], 0.3, "{cmd}");
    }
    assert!(nothing_remembered(&s), "dry runs leave the remembered settings alone");
    assert!(s.execute("modifier.stencilExport", &json!({"id": g.0, "dryRun": 1})).is_err());
}

#[test]
fn without_path_data_or_dry_run_it_returns_what_the_dialog_opens_with() {
    let mut s = session();
    let g = two_tone(&mut s, json!({"material": "card"}));
    let r = ex(&mut s, "modifier.stencilExport", json!({}));
    assert_eq!(r["dialog"], "stencilExport");
    assert_eq!(id_of(&r["id"]), g);
    assert_eq!(r["params"]["kerfMm"], 0.15, "card's kerf: {r}");
    assert!(r.get("files").is_none());
    let r = ex(&mut s, "modifier.stencilTestCard", json!({}));
    assert_eq!(r["dialog"], "stencilTestCard");
    assert_eq!(r["material"], "card", "the selected stencil's material");
    assert_eq!((r["format"].as_str(), r["kind"].as_str()), (Some("svg"), Some("card")));
    let r = ex(&mut s, "modifier.stencilGcode", json!({}));
    assert_eq!((r["dialog"].as_str(), r["format"].as_str()), (Some("stencilExport"), Some("gcode")));
    assert!(r["gcode"]["cut"]["speedMmMin"].is_number(), "{r}");
    assert!(nothing_remembered(&s), "nothing exported, nothing remembered");
}

#[test]
fn kerf_follows_the_material_then_the_params() {
    let mut s = session();
    let g = two_tone(&mut s, json!({"material": "film"}));
    let film = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "dryRun": true}));
    let want = vectorcraft_modifiers::StencilMaterial::Film.limits().unwrap().kerf_mm;
    assert!(close(&film["params"]["kerfMm"], want), "{film}");
    let given = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "dryRun": true, "kerfMm": 0.5}));
    assert!(close(&given["params"]["kerfMm"], 0.5));
    let junk = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "dryRun": true, "kerfMm": 1e9}));
    assert!(junk["params"]["kerfMm"].as_f64().unwrap() <= 2.0, "clamped: {junk}");
}

#[test]
fn a_sheet_bigger_than_the_bed_splits_into_numbered_files() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let dir = tmp("split");
    let path = dir.join("big.svg");
    let r = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "path": path.to_string_lossy(), "bedWidthMm": 200, "bedHeightMm": 200}));
    let n = r["sheets"].as_u64().unwrap();
    assert!(n >= 2, "{r}");
    for (k, f) in r["files"].as_array().unwrap().iter().enumerate() {
        assert_eq!(f["name"], format!("big {}.svg", k + 1));
        assert!(std::path::Path::new(f["path"].as_str().unwrap()).exists());
        assert!(f["wMm"].as_f64().unwrap() <= 200.0 + 1e-6, "{f}");
    }
    assert_eq!(super::laser::sheet_path("a/b.svg", 2, 3), "a/b 2.svg");
    assert_eq!(super::laser::sheet_path("a/b", 2, 3), "a/b 2");
    assert_eq!(super::laser::sheet_path("a/b.svg", 1, 1), "a/b.svg");
}

#[test]
fn what_cannot_be_exported_is_an_error_not_a_panic() {
    let mut s = session();
    let plain = rect(&mut s, 0.0, 0.0, 50.0, 50.0, BLACK);
    for cmd in ["modifier.stencilExport", "modifier.stencilGcode"] {
        assert!(matches!(s.execute(cmd, &json!({"id": plain.0, "data": true})), Err(EngineError::Disabled(..))), "{cmd}");
        assert!(s.execute(cmd, &json!({"id": 999_999, "data": true})).is_err(), "{cmd}");
    }
    // An all-black design cuts nothing (only the registration holes would be: turn them off).
    let a = rect(&mut s, 0.0, 200.0, 50.0, 50.0, BLACK);
    let g = id_of(&ex(&mut s, "modifier.stencilFromSelection", json!({"ids": [a.0], "props": {"hoshi": false}}))["id"]);
    let e = s.execute("modifier.stencilExport", &json!({"id": g.0, "data": true})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)) && e.to_string().contains("nothing to cut"), "{e}");
    let g = two_tone(&mut s, json!({}));
    for p in [json!({"id": g.0, "path": 5}), json!({"id": g.0, "data": "yes"}), json!({"id": g.0, "data": true, "material": "wood"})] {
        assert!(matches!(s.execute("modifier.stencilGcode", &p), Err(EngineError::BadParams { .. })), "{p}");
    }
    // Everything off: refused with the writer's reason, nothing remembered.
    let e = s
        .execute(
            "modifier.stencilGcode",
            &json!({"id": g.0, "data": true, "label": false, "cut": {"enabled": false}, "score": {"enabled": false}, "engrave": {"enabled": false}}),
        )
        .unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)), "{e}");
    assert!(nothing_remembered(&s));
    // Junk settings are clamped, not refused.
    let r = ex(
        &mut s,
        "modifier.stencilGcode",
        json!({"id": g.0, "dryRun": true, "sMax": 1e308, "originXMm": -1e9, "cut": {"powerPct": 900, "passes": -3, "speedMmMin": "x"}}),
    );
    assert_eq!(r["gcode"]["sMax"], 100000.0);
    assert_eq!(r["gcode"]["originXMm"], -5000.0);
    assert_eq!(r["gcode"]["cut"]["powerPct"], 100.0);
}

#[test]
fn stencil_gcode_is_true_size_flipped_and_beam_off_at_both_ends() {
    let mut s = session();
    // A 200 × 120 unit design at 1 mm a unit with a 15 mm margin: the white hole in the top half of
    // the right side, no registration holes.
    let ids =
        [rect(&mut s, 0.0, 0.0, 100.0, 120.0, BLACK), rect(&mut s, 100.0, 60.0, 100.0, 60.0, BLACK), rect(&mut s, 100.0, 0.0, 100.0, 60.0, WHITE)];
    let ids: Vec<u64> = ids.iter().map(|i| i.0).collect();
    let g = id_of(
        &ex(
            &mut s,
            "modifier.stencilFromSelection",
            json!({"ids": ids, "props": {"widthMm": 200, "marginMm": 15, "hoshi": false, "material": "paper"}}),
        )["id"],
    );
    let dir = tmp("sample");
    let path = dir.join("sample.gcode");
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "path": path.to_string_lossy(), "kerfMm": 0.1, "label": false}));
    assert_eq!(r["sheets"], 1, "{r}");
    let f = &r["files"][0];
    assert!(close(&f["wMm"], 230.1) && close(&f["hMm"], 150.1), "the sheet plus the kerf: {f}");
    let b = &f["boundsMm"];
    assert!(close(&b["x0"], 0.0) && close(&b["y0"], 0.0) && close(&b["x1"], 230.1) && close(&b["y1"], 150.1), "{b}");
    assert_eq!(f["name"], "sample.gcode");
    assert!(f["lines"].as_u64().unwrap() > 10 && f["estimatedSec"].as_f64().unwrap() > 0.0, "{f}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert_safe(&text);
    // Y is flipped: the white hole is in the design's top half (sheet y 15..75), so in machine
    // coordinates (y up from the sheet's bottom-left) its edges are near Y 75 and 135.
    let lines: Vec<&str> = text.lines().collect();
    let cut = &lines[lines.iter().position(|l| l.starts_with("M4 S1000")).unwrap()..];
    let hole: Vec<(f64, f64)> = cut.iter().filter_map(|l| xy(l)).filter(|&(x, y)| x > 110.0 && x < 220.0 && y > 5.0 && y < 145.0).collect();
    assert!(!hole.is_empty());
    assert!(hole.iter().all(|&(_, y)| y > 74.0), "{hole:?}");
    // Remembered per material.
    assert!(pref(&s, "machine")["materials"]["paper"]["cut"].is_object(), "{}", s.prefs.picabia_machine);
    assert_eq!(r["gcode"]["sMax"], 1000.0, "the profile's S max: {r}");
    assert_eq!(r["params"]["bedWidthMm"], 584.2, "the machine's bed: {r}");
}

#[test]
fn a_connected_machines_values_win_for_jobs() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    s.picabia.laser_machine = super::LaserMachine { s_max: Some(255.0), travel_x_mm: Some(300.0), travel_y_mm: None };
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "dryRun": true}));
    assert_eq!(r["gcode"]["sMax"], 255.0);
    assert_eq!(r["params"]["bedWidthMm"], 300.0, "the machine's travel is smaller than the profile's bed");
    let p = ex(&mut s, "laser.profile", json!({}));
    assert_eq!((p["sMaxUsed"].as_f64(), p["sMaxMachine"].as_f64()), (Some(255.0), Some(255.0)));
}

#[test]
fn the_test_card_for_each_material_as_svg_and_gcode() {
    let mut s = Session::new();
    for m in ["paper", "film", "card"] {
        let r = ex(&mut s, "modifier.stencilTestCard", json!({"material": m, "data": true}));
        let f = &r["files"][0];
        assert_eq!(f["name"], format!("Test card {m}.svg"));
        assert!((f["wMm"].as_f64().unwrap() - vectorcraft_stencil::CARD_W_MM).abs() < 1.0, "{f}");
        assert!(decode(f).starts_with("<?xml"));
        assert_eq!(r["material"], m);
    }
    // A custom card needs a stencil's limits.
    assert!(matches!(s.execute("modifier.stencilTestCard", &json!({"material": "custom", "data": true})), Err(EngineError::Disabled(..))));
    let r = ex(&mut s, "modifier.stencilTestCard", json!({"format": "gcode", "material": "card", "data": true}));
    assert_eq!(r["kind"], "card");
    let f = &r["files"][0];
    assert_eq!(f["name"], "Test card card.gcode");
    let text = decode(f);
    assert_safe(&text);
    assert!(pref(&s, "machine")["materials"]["card"].is_object(), "the card's settings are remembered");
}

#[test]
fn the_power_ladder_as_gcode() {
    let mut s = session();
    let r = ex(
        &mut s,
        "modifier.stencilTestCard",
        json!({"kind": "powerLadder", "data": true, "powersPct": [10, 50], "speedsMmMin": [100, 1000], "passes": 2}),
    );
    assert_eq!(r["kind"], "powerLadder");
    let f = &r["files"][0];
    assert_eq!(f["name"], "Power ladder paper.gcode");
    let text = decode(f);
    assert_safe(&text);
    for want in ["S100", "S500", "F100", "F1000"] {
        assert!(text.contains(want), "{want}: {text}");
    }
    assert_eq!(r["ladder"]["powersPct"], json!([10.0, 50.0]));
    assert_eq!(s.prefs.picabia_laser, Value::Null, "the ladder's per-square settings aren't remembered");
    let e = s.execute("modifier.stencilTestCard", &json!({"kind": "powerLadder", "format": "svg", "data": true})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)) && e.to_string().contains("G-code"), "{e}");
    for p in [json!({"kind": "ladder", "data": true}), json!({"format": "pdf", "data": true}), json!({"format": 1, "data": true})] {
        assert!(matches!(s.execute("modifier.stencilTestCard", &p), Err(EngineError::BadParams { .. })), "{p}");
    }
    assert!(s.execute("modifier.stencilTestCard", &json!({"kind": "powerLadder", "material": "custom", "data": true})).is_ok());
    let dir = tmp("ladder");
    let path = dir.join("ladder.nc");
    let r = ex(&mut s, "modifier.stencilTestCard", json!({"kind": "powerLadder", "path": path.to_string_lossy()}));
    assert_eq!(r["files"][0]["name"], "ladder.nc");
    assert_safe(&std::fs::read_to_string(&path).unwrap());
}

#[test]
fn the_machine_profile() {
    let mut s = Session::new();
    let r = ex(&mut s, "laser.profile", json!({}));
    let p = &r["profile"];
    assert_eq!((p["bedWidthMm"].as_f64(), p["bedHeightMm"].as_f64()), (Some(584.2), Some(1219.2)));
    assert_eq!(
        (p["sMax"].as_f64(), p["rapidMmMin"].as_f64(), p["air"].as_bool(), p["checked"].as_bool()),
        (Some(1000.0), Some(3000.0), Some(false), Some(false))
    );
    assert_eq!(r["sMaxUsed"], 1000.0);
    assert_eq!(r["sMaxMachine"], Value::Null);
    assert!(r["materialDefaults"]["film"]["cut"].is_object());
    assert!(nothing_remembered(&s), "reading writes nothing");
    let r = ex(
        &mut s,
        "laser.profile",
        json!({"set": {"name": "Shop laser", "bedWidthMm": 1e9, "bedHeightMm": "x", "rapidMmMin": 6000, "materials": {"paper": {"cut": {"passes": 3}}, "wood": {"cut": {}}}}}),
    );
    let p = &r["profile"];
    assert_eq!(p["name"], "Shop laser");
    assert_eq!(p["bedWidthMm"], 5000.0);
    assert_eq!(p["bedHeightMm"], 1219.2);
    assert_eq!(p["materials"]["paper"]["cut"]["passes"], 3);
    assert!(p["materials"].get("wood").is_none());
    assert_eq!(pref(&s, "machine")["rapidMmMin"], 6000.0);
    assert_eq!(r["bedMm"], json!({"w": 5000.0, "h": 1219.2}));
    for bad in [json!({"set": 5}), json!({"set": "x"})] {
        assert!(matches!(s.execute("laser.profile", &bad), Err(EngineError::BadParams { .. })), "{bad}");
    }
    assert_eq!(MachineProfile::from_json(&json!([1, 2])), MachineProfile::default());
    assert_eq!(MachineProfile::from_json(&MachineProfile::default().to_json()), MachineProfile::default());
    // The preferences: null isn't written, older files read as none, and the value survives a
    // round trip and a preferences reset.
    let none = Prefs::default().to_json();
    assert!(none.get("picabiaLaser").is_none() && none.get("picabiaMachine").is_none());
    let old = serde_json::from_value::<Prefs>(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.picabia_laser.is_null() && old.picabia_machine.is_null());
    let saved = s.prefs.to_json();
    assert!(saved["picabiaMachine"].is_object());
    let back: Prefs = serde_json::from_value(saved).unwrap();
    assert_eq!(back.picabia_machine, s.prefs.picabia_machine);
    ex(&mut s, "prefs.reset", json!({}));
    assert!(pref(&s, "machine").is_object(), "a preferences reset keeps the machine profile");
}

/// The (x, y) of the motions burnt at `S` value `s`.
fn op_moves(text: &str, s: u32) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut on = false;
    for l in text.lines() {
        if l.starts_with("M4 ") {
            on = l.split_whitespace().any(|w| w == format!("S{s}"));
        } else if l.starts_with("M5") {
            on = false;
        } else if on && l.starts_with("G1") {
            out.extend(xy(l));
        }
    }
    out
}

#[test]
fn any_objects_export_as_gcode() {
    let mut s = session();
    let outer = rect(&mut s, 0.0, 0.0, 72.0, 72.0, BLACK);
    let inner = rect(&mut s, 18.0, 18.0, 36.0, 36.0, WHITE);
    let line = id_of(&ex(&mut s, "shape.line", json!({"x1": 100, "y1": 36, "x2": 172, "y2": 36}))["id"]);
    let r1 = ex(&mut s, "laser.gcode", json!({"ids": [outer.0, inner.0, line.0], "data": true}));
    let file = &r1["files"][0];
    assert_eq!(file["name"], "Design laser.gcode");
    // 172 × 72 points at 25.4/72 mm each.
    let mpu = 25.4 / 72.0;
    assert!(close(&file["wMm"], 172.0 * mpu) && close(&file["hMm"], 72.0 * mpu), "{file}");
    assert!((r1["mmPerUnit"].as_f64().unwrap() - mpu).abs() < 1e-12);
    let text = decode(file);
    assert_safe(&text);
    // The cut: the inner square first, then the outer one (inner contours first).
    let cut = op_moves(&text, 1000);
    let inner_first = cut.iter().position(|&(x, _)| (x - 18.0 * mpu).abs() < 0.01 || (x - 54.0 * mpu).abs() < 0.01).unwrap();
    let outer_first = cut.iter().position(|&(x, _)| x.abs() < 0.01 || (x - 72.0 * mpu).abs() < 0.01).unwrap();
    assert!(inner_first < outer_first, "{cut:?}");
    // The line: open, with the score settings, at half height.
    let score: Vec<(f64, f64)> = text.lines().filter_map(xy).filter(|&(x, y)| x > 100.0 * mpu - 0.01 && (y - 36.0 * mpu).abs() < 0.01).collect();
    assert!(score.len() >= 2, "{text}");
    // Scaled to a width.
    let r2 = ex(&mut s, "laser.gcode", json!({"ids": [outer.0], "dryRun": true, "widthMm": 100}));
    assert!(close(&r2["files"][0]["wMm"], 100.0) && close(&r2["files"][0]["hMm"], 100.0), "{r2}");
    // A stencil goes the stencil's way; stencil-only params are refused for a design.
    let g = two_tone(&mut s, json!({}));
    let r4 = ex(&mut s, "laser.gcode", json!({"ids": [g.0], "dryRun": true, "kerfMm": 0.2}));
    assert_eq!(r4["sheets"], 1, "{r4}");
    assert!(matches!(s.execute("laser.gcode", &json!({"ids": [outer.0], "data": true, "kerfMm": 0.2})), Err(EngineError::BadParams { .. })));
    // The dialog; nothing selected; junk scales.
    let d = ex(&mut s, "laser.gcode", json!({"ids": [outer.0]}));
    assert_eq!((d["dialog"].as_str(), &d["ids"]), (Some("laserGcode"), &json!([outer.0])));
    ex(&mut s, "select.none", json!({}));
    assert!(matches!(s.execute("laser.gcode", &json!({"data": true})), Err(EngineError::Disabled(..))));
    for p in [
        json!({"ids": [outer.0], "data": true, "mmPerUnit": 0}),
        json!({"ids": [outer.0], "data": true, "widthMm": "wide"}),
        json!({"ids": [outer.0], "data": true, "mmPerUnit": 1e9}),
    ] {
        assert!(matches!(s.execute("laser.gcode", &p), Err(EngineError::BadParams { .. })), "{p}");
    }
    // A hidden object isn't cut.
    ex(&mut s, "select.set", json!({"ids": [inner.0]}));
    ex(&mut s, "object.hide", json!({}));
    let r5 = ex(&mut s, "laser.gcode", json!({"ids": [outer.0, inner.0], "data": true}));
    assert!(op_moves(&decode(&r5["files"][0]), 1000).iter().all(|&(x, _)| (x - 18.0 * mpu).abs() > 0.01));
}

#[test]
fn the_other_remembered_settings_carry_over() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    ex(
        &mut s,
        "modifier.stencilExport",
        json!({"id": g.0, "data": true, "bedWidthMm": 0, "bedHeightMm": 0, "scoreColour": "#123456", "overlapMm": 4}),
    );
    let r = ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "data": true}));
    assert_eq!(r["params"]["bedWidthMm"], 0.0);
    assert_eq!(r["params"]["scoreColour"], "#123456");
    assert_eq!(r["params"]["overlapMm"], 4.0);
    // Prefs written and read back keep them.
    let back: Prefs = serde_json::from_str(&serde_json::to_string(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.picabia_laser, s.prefs.picabia_laser);
}

#[test]
fn several_sheets_are_numbered_gcode_files() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let dir = tmp("gsplit");
    let path = dir.join("big.gcode");
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "path": path.to_string_lossy(), "bedWidthMm": 200, "bedHeightMm": 200}));
    let n = r["sheets"].as_u64().unwrap() as usize;
    assert!(n >= 2, "{r}");
    for (k, f) in r["files"].as_array().unwrap().iter().enumerate() {
        let want = dir.join(format!("big {}.gcode", k + 1));
        assert_eq!(f["path"], json!(want.to_string_lossy()));
        assert_eq!(f["sheet"], format!("{} of {n}", k + 1));
        assert!(f["boundsMm"]["x1"].as_f64().unwrap() <= 200.0 + 1e-6, "{f}");
        assert_safe(&std::fs::read_to_string(&want).unwrap());
    }
    assert!(!path.exists());
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true, "bedWidthMm": 200, "bedHeightMm": 200}));
    assert_eq!(r["files"][1]["name"], "Stencil laser 2.gcode");
}

#[test]
fn op_settings_are_remembered_per_material() {
    let mut s = session();
    let g = two_tone(&mut s, json!({"material": "film"}));
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true}));
    assert_eq!(r["material"], "film");
    let film_start = r["gcode"]["cut"]["speedMmMin"].as_f64().unwrap();
    let faster = film_start + 50.0;
    ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true, "cut": {"speedMmMin": faster, "passes": 4}, "air": true}));
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true}));
    assert_eq!(r["gcode"]["cut"]["speedMmMin"], faster);
    assert_eq!(r["gcode"]["cut"]["passes"], 4);
    assert_eq!(r["gcode"]["air"], false, "air is the profile's, not remembered per export");
    assert!(decode(&r["files"][0]).contains(&format!("F{faster}")), "F{faster}");
    // Paper's are its own.
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true, "material": "paper"}));
    let paper_start = vectorcraft_stencil::GcodeParams::for_material("paper").to_json()["cut"]["speedMmMin"].clone();
    assert_eq!(r["gcode"]["cut"]["speedMmMin"], paper_start);
    assert_eq!(s.prefs.picabia_machine["materials"]["film"]["cut"]["speedMmMin"], faster);
    // The profile's S max and air reach the G-code.
    ex(&mut s, "laser.profile", json!({"set": {"sMax": 255, "air": true}}));
    let r = ex(&mut s, "modifier.stencilGcode", json!({"id": g.0, "data": true}));
    let text = decode(&r["files"][0]);
    assert!(text.contains("M4 S255") && text.contains("\nM8\n") && text.contains("\nM9\n"), "{text}");
    assert_safe(&text);
    let back: Prefs = serde_json::from_str(&serde_json::to_string(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.picabia_machine, s.prefs.picabia_machine);
}

#[test]
fn laser_and_machine_settings_are_separate_preferences() {
    // The app merges two windows' preferences key by key: the laser export's settings and the
    // machine profile are two keys, so one window's export and another's profile edit both stay.
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    let before = s.prefs.to_json();
    ex(&mut s, "modifier.stencilExport", json!({"id": g.0, "data": true}));
    let after_export = s.prefs.to_json();
    ex(&mut s, "laser.profile", json!({"set": {"name": "Shop"}}));
    let after_profile = s.prefs.to_json();
    let changed = |a: &Value, b: &Value| -> Vec<String> {
        let (a, b) = (a.as_object().unwrap(), b.as_object().unwrap());
        let mut keys: Vec<String> = a.keys().chain(b.keys()).filter(|k| a.get(*k) != b.get(*k)).cloned().collect();
        keys.sort();
        keys.dedup();
        keys
    };
    assert_eq!(changed(&before, &after_export), ["picabiaLaser"]);
    assert_eq!(changed(&after_export, &after_profile), ["picabiaMachine"]);
}

#[test]
fn a_huge_design_is_refused_before_it_is_flattened() {
    let mut s = session();
    // The canvas caps coordinates, so the size comes from the scale: 1500 points at 100 mm each, and
    // a circle (curves flattened to 0.01 mm would be millions of points) beside it.
    let a = rect(&mut s, 0.0, 0.0, 1500.0, 1500.0, BLACK);
    let c = id_of(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 1500, "height": 1500}))["id"]);
    let e = s.execute("laser.gcode", &json!({"ids": [a.0, c.0], "data": true, "mmPerUnit": 100})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)) && e.to_string().contains("at most"), "{e}");
    // Scaled down to a width, it cuts.
    let r = ex(&mut s, "laser.gcode", json!({"ids": [a.0], "dryRun": true, "widthMm": 100}));
    assert!(close(&r["files"][0]["wMm"], 100.0), "{r}");
}

/// Every G-code reply counts the lines a job sends: the header's comment lines don't count, so
/// the Laser window's dry run and the job agree (stencil, test card, power ladder, any design).
#[test]
fn gcode_line_counts_are_the_lines_a_job_sends() {
    let mut s = session();
    let g = two_tone(&mut s, json!({}));
    for (cmd, p) in [
        ("modifier.stencilGcode", json!({"id": g.0, "data": true})),
        ("modifier.stencilTestCard", json!({"format": "gcode", "data": true})),
        ("modifier.stencilTestCard", json!({"kind": "powerLadder", "data": true})),
        ("laser.gcode", json!({"ids": [g.0], "data": true})),
    ] {
        let r = ex(&mut s, cmd, p);
        let f = &r["files"][0];
        let text = decode(f);
        let sent = crate::cmd::picabia_laser_link::check_gcode(&text).unwrap().len() as u64;
        assert_eq!(f["lines"].as_u64(), Some(sent), "{cmd}");
        assert!(text.lines().count() as u64 > sent, "{cmd}: the header's comments aren't counted");
    }
}
