//! PICABIA: the every-command junk sweep for the `physics.*` commands on objects that already
//! carry physics settings (the general sweep in `command_sweep.rs` runs them on plain fixtures):
//! each call returns Ok or Err, never panics, never leaves an interaction open and never corrupts
//! the tree; Simulate undoes exactly.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_engine::{Session, command_specs};
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::Fixture;
use vectorcraft_testkit::invariants::check_session;
use vectorcraft_testkit::strategies::junk_params;

/// `fx` with physics settings on each selected object (alternately dynamic and static, every
/// collider kind), and one object's settings damaged.
fn with_physics(fx: Fixture) -> Session {
    let mut s = fx.session();
    let selected: Vec<u64> = s.active().map(|d| d.selection.objects.iter().map(|i| i.0).collect()).unwrap_or_default();
    let kinds = ["auto", "precise", "fast", "box"];
    for (k, id) in selected.iter().enumerate() {
        let body = ["dynamic", "static", "soft"][k % 3];
        let preset = ["cloth", "jelly", "rubber"][k % 3];
        let soft = json!({"preset": preset, "pins": [{"u": 0, "v": 0.5}]});
        let _ = s.execute(
            "physics.set",
            &json!({"ids": [id], "props": {"bodyType": body, "colliderType": kinds[k % 4], "restitution": 0.6, "soft": soft}}),
        );
    }
    s
}

#[test]
fn physics_commands_with_junk_params_on_objects_with_settings() {
    let path = vectorcraft_testkit::temp_dir("picabia-physics-sweep").join("out.bin").to_string_lossy().to_string();
    let mut failures = vec![];
    let mut calls = 0usize;
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("physics.")).collect();
    assert_eq!(ids.len(), 15, "{ids:?}");
    for fx in Fixture::ALL {
        for id in &ids {
            let spec = command_specs().iter().find(|c| c.id == *id).unwrap();
            let mut cases = junk_params(spec.params, &path);
            cases.push(json!({}));
            cases.push(json!({"steps": 20000, "walls": false, "gravityX": -100000, "gravityY": 100000}));
            cases.push(json!({"props": {"bodyType": "soft", "colliderType": 7, "mass": -1, "sensor": true}}));
            cases.push(json!({"props": {"bodyType": "soft", "soft": {"preset": "cloth", "stiffness": "x", "resolution": 1e300, "pins": [{"u": 1e300, "v": -1, "attachTo": 1}, 3]}}}));
            cases.push(json!({"props": {"soft": [1], "bodyType": "soft"}}));
            cases.push(json!({"step": 36_000}));
            cases.push(json!({"id": 1, "point": [0, 0], "to": {"x": 1e7, "y": -1e7}, "release": true}));
            for p in &cases {
                calls += 1;
                let mut s = with_physics(fx);
                // Session commands need a session: half the cases open one first.
                if calls.is_multiple_of(2) {
                    let _ = s.execute("physics.session.open", &json!({}));
                    let _ = s.execute("physics.seek", &json!({"step": 20}));
                }
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
    assert!(calls > 50, "only {calls} calls");
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} failures (of {calls} calls):\n{}",
        failures.len(),
        failures.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The session on every fixture: play by ticks, poke, Apply undoes exactly, Cancel changes nothing.
#[test]
fn sessions_apply_and_undo_exactly_on_every_fixture() {
    for fx in Fixture::ALL {
        let mut s = with_physics(fx);
        let Some(before) = s.active().map(|d| d.doc.clone()) else { continue };
        let undo = s.active().map_or(0, |d| d.history.undo.len());
        if s.execute("physics.session.open", &json!({})).is_err() {
            continue;
        }
        let _ = s.execute("physics.play", &json!({}));
        for _ in 0..30 {
            vectorcraft_engine::cmd::picabia_physics::session::tick(&mut s, 1.0 / 30.0);
        }
        let _ = s.execute("physics.cancel", &json!({}));
        assert!(s.active().unwrap().doc == before, "[{fx:?}] cancel changed the document");
        let _ = s.execute("physics.toRest", &json!({"steps": 300}));
        if s.execute("physics.apply", &json!({})).is_ok() && s.active().map_or(0, |d| d.history.undo.len()) > undo {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert!(s.active().unwrap().doc == before, "[{fx:?}] undo after apply");
        check_session(&s).unwrap();
    }
}

/// Simulate on every fixture undoes exactly.
#[test]
fn simulate_undoes_exactly_on_every_fixture() {
    for fx in Fixture::ALL {
        let mut s = with_physics(fx);
        let Some(before) = s.active().map(|d| d.doc.clone()) else { continue };
        let undo = s.active().map_or(0, |d| d.history.undo.len());
        if s.execute("physics.simulate", &json!({"steps": 300})).is_err() {
            continue;
        }
        if s.active().map_or(0, |d| d.history.undo.len()) > undo {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert!(s.active().unwrap().doc == before, "[{fx:?}] undo after simulate");
    }
}
