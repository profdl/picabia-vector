//! PICABIA: the every-command junk sweep for the `modifier.*` commands on objects that already
//! carry modifier stacks (the general sweep in `command_sweep.rs` runs them on plain fixtures):
//! each call returns Ok or Err, never panics, never leaves an interaction open and never corrupts
//! the tree.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::{Session, command_specs};
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::Fixture;
use vectorcraft_testkit::invariants::check_session;
use vectorcraft_testkit::strategies::junk_params;

/// `fx` with a stack on each selected object: a resample (geometry, on the object), a grid array,
/// a trim, a tidy, a deform and a displace (geometry, on every copy), a mirror and a copy to shape onto the first
/// selected object (refused on that object itself), and copied rows for `modifier.paste`.
fn stacked(fx: Fixture) -> Session {
    let mut s = fx.session();
    let selected: Vec<u64> = s.active().map(|d| d.selection.objects.iter().map(|i| i.0).collect()).unwrap_or_default();
    for (k, id) in selected.iter().enumerate() {
        s.execute("modifier.add", &json!({"ids": [id], "kind": "resample", "props": {"mode": "subdivide", "iterations": 2}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "gridArray", "props": {"rows": 2, "cols": 2}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "trim", "props": {"start": 0.2, "end": 0.7, "offset": 0.4}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "tidy", "props": {"close": "smooth", "sort": "random", "reverse": true}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "deform", "props": {"mode": "twist", "angle": 270, "falloff": "smooth"}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "displace", "props": {"mode": "radial", "amplitude": 20}})).unwrap();
        s.execute("modifier.add", &json!({"ids": [id], "kind": "mirror"})).unwrap();
        if k > 0 {
            let _ = s.execute("modifier.add", &json!({"ids": [id], "kind": "copyToShape", "props": {"target": selected[0].to_string()}}));
        }
    }
    if let Some(first) = selected.first() {
        s.execute("modifier.copy", &json!({"id": first})).unwrap();
    }
    s
}

#[test]
fn modifier_commands_with_junk_params_on_stacked_objects() {
    let path = vectorcraft_testkit::temp_dir("picabia-sweep").join("out.bin").to_string_lossy().to_string();
    let mut failures = vec![];
    let mut calls = 0usize;
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("modifier.")).collect();
    assert!(ids.len() >= 9, "{ids:?}");
    for fx in Fixture::ALL {
        for id in &ids {
            let spec = command_specs().iter().find(|c| c.id == *id).unwrap();
            let mut cases = junk_params(spec.params, &path);
            cases.push(json!({}));
            for p in &cases {
                calls += 1;
                let mut s = stacked(fx);
                let r = catch_quiet(|| s.execute(id, p));
                let fail = match r {
                    Err(msg) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                    Ok(Err(vectorcraft_engine::EngineError::Internal { msg, .. })) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                    Ok(_) if s.in_interaction() => Some(format!("{id} {p} [{fx:?}]: left an interaction open")),
                    Ok(_) => check_session(&s).err().map(|e| format!("{id} {p} [{fx:?}]: {e}")),
                };
                failures.extend(fail);
            }
        }
    }
    assert!(calls > 100, "only {calls} calls");
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} failures (of {calls} calls):\n{}",
        failures.len(),
        failures.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// Every editing `modifier.*` command undoes exactly, on stacked objects.
#[test]
fn modifier_commands_undo_exactly() {
    let cases: Vec<(&str, Value)> = vec![
        ("modifier.add", json!({"kind": "linearArray", "props": {"count": 3}})),
        ("modifier.remove", json!({"index": 0})),
        ("modifier.move", json!({"from": 0, "to": 1})),
        ("modifier.set", json!({"index": 1, "props": {"rows": 3}})),
        ("modifier.set", json!({"index": 2, "props": {"start": 0.5}})),
        ("modifier.toggle", json!({"index": 1})),
        ("modifier.paste", json!({"replace": true})),
        ("modifier.remove_all", json!({})),
        ("modifier.apply", json!({})),
        ("modifier.duplicate", json!({"index": 0})),
        ("modifier.apply", json!({"upTo": 0})),
    ];
    for (id, p) in cases {
        let mut s = stacked(Fixture::Multi);
        let before = vectorcraft_testkit::invariants::doc_json(&s.active().unwrap().doc);
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        s.execute("edit.undo", &json!({})).unwrap();
        let after = vectorcraft_testkit::invariants::doc_json(&s.active().unwrap().doc);
        assert!(before == after, "{id}: {}", vectorcraft_testkit::invariants::first_diff(&before, &after, "$"));
    }
}

/// Each geometry row kind `modifier.add` takes, each param set to each junk value in turn, added
/// on top of stacked objects, then listed and applied: Ok or Err, never a panic or a corrupt tree.
#[test]
fn geometry_kinds_with_junk_props() {
    let junk = vectorcraft_testkit::strategies::junk_values();
    let mut failures = vec![];
    let mut calls = 0usize;
    // The geometry rows (the other kinds' params are swept through `modifier.set` above and in the
    // modifiers crate's clamp tests).
    let kinds: Vec<&str> = vectorcraft_modifiers::KINDS
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| vectorcraft_modifiers::Modifier::new("m1", k).unwrap().kind.is_geometry())
        .collect();
    assert_eq!(kinds, ["resample", "trim", "tidy", "deform", "displace"]);
    for kind in kinds {
        let params: Vec<String> = vectorcraft_modifiers::Modifier::new("m1", kind).unwrap().kind.params().keys().cloned().collect();
        for fx in [Fixture::Single, Fixture::Multi] {
            for key in &params {
                for v in &junk {
                    calls += 1;
                    let mut s = stacked(fx);
                    let r = catch_quiet(|| {
                        let _ = s.execute("modifier.add", &json!({"kind": kind, "index": 0, "props": {key.as_str(): v}}));
                        let _ = s.execute("modifier.list", &json!({}));
                        s.execute("modifier.apply", &json!({}))
                    });
                    let fail = match r {
                        Err(msg) => Some(format!("PANIC {kind}.{key} = {v} [{fx:?}]: {msg}")),
                        Ok(Err(vectorcraft_engine::EngineError::Internal { msg, .. })) => Some(format!("PANIC {kind}.{key} = {v} [{fx:?}]: {msg}")),
                        Ok(_) => check_session(&s).err().map(|e| format!("{kind}.{key} = {v} [{fx:?}]: {e}")),
                    };
                    failures.extend(fail);
                }
            }
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

/// `fx` with a stencil: a black and a white square (and the fixture's selection) under one
/// stencil row, selected, plus a plain square: the stencil and laser commands find something to
/// compute and cut.
fn stenciled(fx: Fixture) -> Session {
    let mut s = fx.session();
    if s.active().is_none() {
        return s;
    }
    let square = |s: &mut Session, x: f64, hex: &str| {
        let id = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [id], "color": hex})).unwrap();
        id
    };
    let a = square(&mut s, 0.0, "#000000");
    let b = square(&mut s, 40.0, "#ffffff");
    square(&mut s, 200.0, "#808080");
    s.execute("modifier.stencilFromSelection", &json!({"ids": [a, b], "props": {"widthMm": 80}})).unwrap();
    s
}

/// Every stencil and laser command with junk params, on a document with a stencil (and on the
/// plain fixtures): Ok or Err, never a panic, an open interaction or a corrupt tree. File params
/// get a path in a temporary folder. Nothing here opens a port: these commands only write text.
#[test]
fn stencil_and_laser_commands_with_junk_params() {
    let dir = vectorcraft_testkit::temp_dir("picabia-laser-sweep");
    let path = dir.join("out.gcode").to_string_lossy().to_string();
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("modifier.stencil") || id.starts_with("laser.")).collect();
    for want in [
        "modifier.stencilInfo",
        "modifier.stencilFromSelection",
        "modifier.stencilExport",
        "modifier.stencilTestCard",
        "modifier.stencilGcode",
        "laser.gcode",
        "laser.profile",
    ] {
        assert!(ids.contains(&want), "{want} registered: {ids:?}");
    }
    let mut failures = vec![];
    let mut calls = 0usize;
    let mut gcode_files = 0usize;
    for fx in Fixture::ALL {
        for id in &ids {
            let spec = command_specs().iter().find(|c| c.id == *id).unwrap();
            let mut cases = junk_params(spec.params, &path);
            cases.push(json!({}));
            cases.push(json!({"dryRun": true}));
            cases.push(json!({"data": true, "kind": "powerLadder"}));
            cases.push(json!({"data": true, "format": "gcode"}));
            // Each junk value again with `data: true`, so the junk reaches the G-code writer.
            let with_data: Vec<Value> = cases
                .iter()
                .filter_map(|p| {
                    let mut o = p.as_object()?.clone();
                    if o.contains_key("path") || o.contains_key("dryRun") || o.contains_key("data") {
                        return None;
                    }
                    o.insert("data".into(), json!(true));
                    Some(Value::Object(o))
                })
                .collect();
            cases.extend(with_data);
            for p in &cases {
                calls += 1;
                let _ = std::fs::remove_file(&path);
                let mut s = stenciled(fx);
                let r = catch_quiet(|| s.execute(id, p));
                let fail = match r {
                    Err(msg) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                    Ok(Err(vectorcraft_engine::EngineError::Internal { msg, .. })) => Some(format!("PANIC {id} {p} [{fx:?}]: {msg}")),
                    Ok(_) if s.in_interaction() => Some(format!("{id} {p} [{fx:?}]: left an interaction open")),
                    Ok(Ok(v)) => gcode_checks(&v, &mut gcode_files)
                        .err()
                        .map(|e| format!("UNSAFE G-code {id} {p} [{fx:?}]: {e}"))
                        .or_else(|| check_session(&s).err().map(|e| format!("{id} {p} [{fx:?}]: {e}"))),
                    Ok(Err(_)) => check_session(&s).err().map(|e| format!("{id} {p} [{fx:?}]: {e}")),
                };
                failures.extend(fail);
            }
        }
    }
    assert!(calls > 100, "only {calls} calls");
    assert!(gcode_files > 20, "only {gcode_files} G-code files checked");
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} failures (of {calls} calls):\n{}",
        failures.len(),
        failures.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The X and Y of a motion line (`G0`/`G1`).
fn xy(line: &str) -> Option<(f64, f64)> {
    if !(line.starts_with("G0 ") || line.starts_with("G1 ")) {
        return None;
    }
    let word = |c: char| line.split_whitespace().find_map(|w| w.strip_prefix(c)).and_then(|v| v.parse::<f64>().ok());
    Some((word('X')?, word('Y')?))
}

/// The writer's safety rules on one G-code text: laser mode with dynamic power only (`M4`, never
/// `M3`), mm and absolute, beam off before the first motion and at the end, first and last
/// motions travels, beam off before the park, every coordinate finite.
fn gcode_safe(text: &str) -> Result<(), String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.starts_with(';') && !l.is_empty()).collect();
    if lines.iter().any(|l| l.starts_with("M3")) {
        return Err("M3 (constant power)".into());
    }
    if !(lines.contains(&"G21") && lines.contains(&"G90")) {
        return Err("no G21/G90".into());
    }
    if lines.iter().any(|l| l.contains("NaN") || l.contains("inf")) {
        return Err("a non-finite number".into());
    }
    let Some(first) = lines.iter().position(|l| xy(l).is_some()) else { return Err("no motion".into()) };
    if !lines.get(..first).is_some_and(|b| b.contains(&"M5")) {
        return Err("beam not off before the first motion".into());
    }
    let last = lines.iter().rposition(|l| xy(l).is_some()).unwrap_or(first);
    if !(lines[first].starts_with("G0") && lines[last].starts_with("G0")) {
        return Err("the first or last motion burns".into());
    }
    if lines.last() != Some(&"M5") {
        return Err("beam not off at the end".into());
    }
    if let Some(burn) = lines.iter().rposition(|l| l.starts_with("G1"))
        && !lines.get(burn..last).is_some_and(|b| b.contains(&"M5"))
    {
        return Err("beam not off before the park".into());
    }
    Ok(())
}

/// Every G-code file a command's result carries (base64 `data` or a written `path`) is safe.
fn gcode_checks(v: &Value, count: &mut usize) -> Result<(), String> {
    for f in v["files"].as_array().into_iter().flatten() {
        let text = if let Some(d) = f["data"].as_str() {
            String::from_utf8(vectorcraft_format::base64_decode(d).ok_or("bad base64")?).map_err(|e| e.to_string())?
        } else if let Some(p) = f["path"].as_str() {
            std::fs::read_to_string(p).map_err(|e| e.to_string())?
        } else {
            continue;
        };
        if text.contains("\nG21\n") || text.starts_with("G21") {
            *count += 1;
            gcode_safe(&text)?;
        }
    }
    Ok(())
}

/// `modifier.stencilFromSelection` undoes exactly (the group and the row in one step).
#[test]
fn stencil_from_selection_undoes_exactly() {
    let mut s = Fixture::Multi.session();
    let before = vectorcraft_testkit::invariants::doc_json(&s.active().unwrap().doc);
    s.execute("modifier.stencilFromSelection", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let after = vectorcraft_testkit::invariants::doc_json(&s.active().unwrap().doc);
    assert!(before == after, "{}", vectorcraft_testkit::invariants::first_diff(&before, &after, "$"));
}
