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

/// `fx` with a stack on each selected object: a grid array, a mirror and a copy to shape onto the
/// first selected object (refused on that object itself), and copied rows for `modifier.paste`.
fn stacked(fx: Fixture) -> Session {
    let mut s = fx.session();
    let selected: Vec<u64> = s.active().map(|d| d.selection.objects.iter().map(|i| i.0).collect()).unwrap_or_default();
    for (k, id) in selected.iter().enumerate() {
        s.execute("modifier.add", &json!({"ids": [id], "kind": "gridArray", "props": {"rows": 2, "cols": 2}})).unwrap();
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
        ("modifier.set", json!({"index": 0, "props": {"rows": 3}})),
        ("modifier.toggle", json!({"index": 1})),
        ("modifier.paste", json!({"replace": true})),
        ("modifier.remove_all", json!({})),
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
