//! PICABIA: modifier stack commands, and the stack (a foreign attribute) surviving save and
//! reopen, copy and paste, duplicate, Alt-drag copies and undo (SVG: `export_tests`).

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::{EngineError, Session};

pub(super) fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 2000, "height": 2000})).unwrap();
    s
}

pub(super) fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

pub(super) fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    match s.execute(cmd, &p) {
        Err(e) => e.to_string(),
        Ok(v) => panic!("{cmd} {p} ran: {v}"),
    }
}

pub(super) fn id_of(v: &Value) -> NodeId {
    NodeId(v.as_u64().unwrap_or_else(|| panic!("not an id: {v}")))
}

pub(super) fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"])
}

/// Object `id`'s stack source.
pub(super) fn data(s: &Session, id: NodeId) -> Option<String> {
    vectorcraft_modifiers::node_source(s.doc().unwrap().doc.node(id).unwrap()).map(str::to_string)
}

/// Put `src` on object `id` as its stack source, as a file could have it.
pub(super) fn put(s: &mut Session, id: NodeId, src: &str) {
    s.edit("Put", |d, _| {
        vectorcraft_modifiers::set_node_source(d.node_mut(id).unwrap(), Some(src.to_string()));
        Ok(())
    })
    .unwrap();
}

pub(super) fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

pub(super) fn kinds(s: &mut Session, id: NodeId) -> Vec<String> {
    let l = ex(s, "modifier.list", json!({"id": id.0}));
    l["modifiers"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap().to_string()).collect()
}

/// A rectangle with a 5-copy linear array.
pub(super) fn arrayed() -> (Session, NodeId) {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    let out = ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"count": 5, "offsetX": 120}}));
    assert_eq!(out["modifier"], "m1");
    (s, r)
}

#[test]
fn add_set_toggle_move_remove_with_undo() {
    let (mut s, r) = arrayed();
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["instances"], 5);
    assert_eq!(l["modifiers"][0]["count"], 5);
    assert_eq!(l["modifiers"][0]["label"], "Linear array");
    assert_eq!(undo_labels(&s).last().unwrap(), "Add Modifier");
    // The selection is the default target.
    ex(&mut s, "select.set", json!({"ids": [r.0]}));
    ex(&mut s, "modifier.add", json!({"kind": "mirror", "index": 0}));
    assert_eq!(kinds(&mut s, r), ["mirror", "linearArray"]);
    ex(&mut s, "modifier.move", json!({"from": 0, "to": 1}));
    assert_eq!(kinds(&mut s, r), ["linearArray", "mirror"]);
    ex(&mut s, "modifier.set", json!({"modifier": "m1", "props": {"count": 8, "offsetY": "10"}}));
    // 8 copies, then the mirror (keepOriginal) adds the reflected block: 16.
    assert_eq!(ex(&mut s, "modifier.list", json!({}))["instances"], 16);
    assert_eq!(undo_labels(&s).last().unwrap(), "Set Modifier");
    ex(&mut s, "modifier.toggle", json!({"index": 0}));
    let l = ex(&mut s, "modifier.list", json!({}));
    assert_eq!(l["modifiers"][0]["enabled"], json!(false), "{l}");
    assert_ne!(l["instances"], 16);
    ex(&mut s, "modifier.toggle", json!({"index": 0, "enabled": true}));
    ex(&mut s, "modifier.remove", json!({"modifier": "m2"}));
    assert_eq!(kinds(&mut s, r), ["linearArray"]);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(kinds(&mut s, r), ["linearArray", "mirror"]);
    ex(&mut s, "modifier.remove_all", json!({}));
    assert_eq!(data(&s, r), None, "an empty stack leaves no data");
    ex(&mut s, "edit.undo", json!({}));
    assert!(data(&s, r).is_some());
    // Removing the last row removes the attribute too.
    ex(&mut s, "modifier.remove", json!({"index": 1}));
    ex(&mut s, "modifier.remove", json!({"index": 0}));
    assert_eq!(data(&s, r), None);
    assert!(s.doc().unwrap().doc.node(r).unwrap().extras().attrs.is_none(), "no empty attributes left behind");
}

#[test]
fn bad_params_are_refused_and_roll_back() {
    let (mut s, r) = arrayed();
    let before = data(&s, r);
    let undo = undo_labels(&s).len();
    for (cmd, p, want) in [
        ("modifier.add", json!({"ids": [r.0], "kind": "spiralArray"}), "unknown kind"),
        ("modifier.add", json!({"ids": [r.0]}), "`kind` required"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"offsetX": "NaN"}}), "finite"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"offsetX": "-inf"}}), "finite"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"nope": 1}}), "no param `nope`"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": 5}), "object"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "index": -1}), "non-negative integer"),
        ("modifier.set", json!({"ids": [r.0], "modifier": "m1", "props": {"count": "Infinity"}}), "finite"),
        ("modifier.set", json!({"ids": [r.0], "modifier": "m9", "props": {}}), "no modifier `m9`"),
        ("modifier.set", json!({"ids": [r.0], "index": 3, "props": {}}), "no modifier at index 3"),
        ("modifier.set", json!({"ids": [r.0], "modifier": "m1"}), "`props` required"),
        ("modifier.remove", json!({"ids": [r.0]}), "required"),
        ("modifier.remove", json!({"ids": [r.0], "modifier": "m1", "index": 0}), "not both"),
        ("modifier.move", json!({"ids": [r.0], "from": 0, "to": 4}), "1 modifiers"),
        ("modifier.toggle", json!({"ids": [r.0], "index": 0, "enabled": "maybe"}), "true or false"),
        ("modifier.add", json!({"ids": [r.0], "kind": "linearArray", "bogus": 1}), "unknown parameter `bogus`"),
        ("modifier.add", json!({"ids": [999_999], "kind": "mirror"}), "999999"),
    ] {
        let e = err(&mut s, cmd, p.clone());
        assert!(e.contains(want), "{cmd} {p}: {e}");
        assert_eq!(data(&s, r), before, "{cmd} {p} changed nothing");
    }
    assert_eq!(undo_labels(&s).len(), undo, "no undo steps");
    let layer = s.doc().unwrap().doc.layers[0].id;
    assert!(err(&mut s, "modifier.add", json!({"id": layer.0, "kind": "mirror"})).contains("layer"));
}

#[test]
fn several_objects_edit_in_one_step_and_a_refusal_rolls_back_all() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 300.0));
    let out = ex(&mut s, "modifier.add", json!({"ids": [a.0, b.0], "kind": "gridArray", "props": {"rows": 2, "cols": 3}}));
    assert_eq!(out["added"], json!([{"id": a.0, "modifier": "m1"}, {"id": b.0, "modifier": "m1"}]));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": b.0}))["instances"], 6);
    ex(&mut s, "modifier.add", json!({"ids": [a.0], "kind": "mirror"}));
    // `index` 1 exists on `a` only: refused on `b`, and `a` keeps its stack too.
    let before = (data(&s, a), data(&s, b));
    err(&mut s, "modifier.remove", json!({"ids": [a.0, b.0], "index": 1}));
    assert_eq!((data(&s, a), data(&s, b)), before);
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!((data(&s, a), data(&s, b)), (None, None), "one step for both");
}

#[test]
fn damaged_data_is_kept_until_cleared() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    put(&mut s, r, "{broken");
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert!(l["error"].as_str().unwrap().contains("JSON"), "{l}");
    assert!(err(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror"})).contains("remove_all"));
    assert_eq!(data(&s, r).as_deref(), Some("{broken"));
    ex(&mut s, "modifier.remove_all", json!({"ids": [r.0]}));
    assert_eq!(data(&s, r), None);
}

#[test]
fn the_stack_is_capped_at_64() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    let steps: Vec<Value> = (0..64).map(|_| json!({"command": "modifier.add", "params": {"ids": [r.0], "kind": "mirror"}})).collect();
    ex(&mut s, "command.batch", json!({"commands": steps}));
    assert!(err(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror"})).contains("at most 64"));
}

#[test]
fn copy_and_paste_modifiers() {
    let (mut s, r) = arrayed();
    let other = rect(&mut s, 500.0);
    ex(&mut s, "modifier.add", json!({"ids": [other.0], "kind": "randomize"}));
    assert!(matches!(s.execute("modifier.paste", &json!({"ids": [other.0]})), Err(EngineError::Disabled(..))));
    assert_eq!(ex(&mut s, "modifier.copy", json!({"id": r.0}))["copied"], 1);
    assert_eq!(ex(&mut s, "modifier.paste", json!({"ids": [other.0]}))["pasted"], 1);
    let l = ex(&mut s, "modifier.list", json!({"id": other.0}));
    assert_eq!(kinds(&mut s, other), ["randomize", "linearArray"]);
    assert_eq!(l["modifiers"][1]["id"], "m2", "a fresh id where m1 was taken");
    assert_eq!(undo_labels(&s).last().unwrap(), "Paste Modifiers");
    ex(&mut s, "modifier.paste", json!({"ids": [other.0], "replace": true}));
    assert_eq!(kinds(&mut s, other), ["linearArray"]);
    // One row by id; copying objects afterwards keeps the copied rows.
    ex(&mut s, "modifier.copy", json!({"id": other.0, "modifier": "m1"}));
    ex(&mut s, "select.set", json!({"ids": [r.0]}));
    ex(&mut s, "edit.copy", json!({}));
    assert!(s.execute("modifier.paste", &json!({"ids": [r.0]})).is_ok());
    // Nothing to copy.
    let plain = rect(&mut s, 900.0);
    assert!(matches!(s.execute("modifier.copy", &json!({"id": plain.0})), Err(EngineError::Disabled(..))));
}

#[test]
fn the_stack_survives_object_copy_paste_duplicate_and_copying_moves() {
    let (mut s, r) = arrayed();
    let want = data(&s, r).unwrap();
    ex(&mut s, "select.set", json!({"ids": [r.0]}));
    ex(&mut s, "edit.copy", json!({}));
    let pasted = id_of(&ex(&mut s, "edit.paste", json!({}))["ids"][0]);
    assert_ne!(pasted, r);
    assert_eq!(data(&s, pasted).as_deref(), Some(want.as_str()), "copy/paste");
    let dup = id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [r.0]}))["ids"][0]);
    assert_eq!(data(&s, dup).as_deref(), Some(want.as_str()), "duplicate");
    // Alt-drag and Move… with Copy make their copies the same way.
    let moved = id_of(&ex(&mut s, "object.move", json!({"ids": [r.0], "dx": 300, "dy": 0, "copy": true}))["ids"][0]);
    assert_ne!(moved, r);
    assert_eq!(data(&s, moved).as_deref(), Some(want.as_str()), "a copying move");
    // Undo takes a stack away with its object, and redo brings it back.
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.node(moved).is_none());
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!(data(&s, moved).as_deref(), Some(want.as_str()));
}

#[test]
fn the_stack_survives_save_and_reopen() {
    let (mut s, r) = arrayed();
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "circularArray", "props": {"radius": 40}}));
    // Fields and kinds this build doesn't know are kept too.
    let mut v: Value = serde_json::from_str(&data(&s, r).unwrap()).unwrap();
    v["stack"].as_array_mut().unwrap().push(json!({"id": "m9", "enabled": true, "kind": "futureKind", "x": [1]}));
    v["stack"][0]["wobble"] = json!(3);
    let src = v.to_string();
    put(&mut s, r, &src);
    let saved = ex(&mut s, "document.serialize", json!({"format": "vectorcraft"}));
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "stack.vectorcraft", "dataBase64": saved["dataBase64"]}));
    let back = data(&t, r).expect("the stack reopens");
    assert_eq!(vectorcraft_modifiers::parse(&back).unwrap(), vectorcraft_modifiers::parse(&src).unwrap());
    let l = ex(&mut t, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["modifiers"][2]["kind"], "futureKind");
    assert_eq!(l["modifiers"][0]["wobble"], 3);
    assert!(l["warnings"].as_array().unwrap().iter().any(|w| w["modifier"] == "m9"), "{l}");
    // The format crate alone keeps it byte for byte.
    let doc = &s.doc().unwrap().doc;
    let again = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(vectorcraft_modifiers::node_source(again.node(r).unwrap()), Some(src.as_str()));
}

#[test]
fn every_modifier_command_documents_its_params() {
    for (id, names) in [
        ("modifier.add", vec!["ids", "id", "kind", "props", "index"]),
        ("modifier.remove", vec!["ids", "id", "modifier", "index"]),
        ("modifier.move", vec!["ids", "id", "from", "to"]),
        ("modifier.set", vec!["ids", "id", "modifier", "index", "props"]),
        ("modifier.toggle", vec!["ids", "id", "modifier", "index", "enabled"]),
        ("modifier.copy", vec!["id", "modifier", "index"]),
        ("modifier.paste", vec!["ids", "id", "replace"]),
        ("modifier.list", vec!["id"]),
        ("modifier.remove_all", vec!["ids", "id"]),
        ("modifier.apply", vec!["ids", "id", "upTo"]),
        ("modifier.duplicate", vec!["ids", "id", "modifier", "index"]),
        ("modifier.targets", vec!["id", "modifier", "index"]),
    ] {
        let spec = crate::cmd::find_command(id).unwrap();
        assert_eq!(crate::cmd::params::doc_keys(spec.params).unwrap().keys, names, "{id}");
    }
}

/// Modifier-specific junk on every `modifier.*` command, on a stack and on damaged data (the
/// general sweep in `tests/command_sweep.rs` covers them with its own junk too): nothing panics.
#[test]
fn modifier_commands_survive_junk() {
    let junk = [
        json!({"kind": "linearArray", "props": {"count": 1e308, "offsetX": 1e308, "rotateEach": -1e308, "scaleEach": 1e308}}),
        json!({"kind": "linearArray", "props": {"count": "NaN", "offsetX": "inf"}}),
        json!({"kind": [1], "props": [1], "index": -1, "modifier": 5}),
        json!({"kind": "\u{0}", "props": {"\u{0}": null}, "index": 1e308, "modifier": "\u{0}"}),
        json!({"from": -1, "to": 1e308, "enabled": [1], "replace": {"x": 1}}),
        json!({"from": "x", "to": null, "enabled": null, "modifier": null, "index": null}),
        json!({"from": 0, "to": 0, "index": 18_446_744_073_709_551_615_u64}),
        json!({"ids": [{"x": 1}], "kind": "mirror", "props": {"axis": 7, "keepOriginal": "x"}}),
        json!({"kind": "gridArray", "props": {"rows": -5, "cols": 1e9, "gapX": "1e400"}}),
        json!({"kind": "randomize", "props": {"seed": -1, "scale": "x".repeat(10_000)}}),
        json!({"kind": "transformEach", "props": {"rotateExpr": "i * ", "scaleExpr": "x".repeat(10_000)}}),
        json!({"kind": "copyToShape", "props": {"target": "\u{0}", "mode": 7, "count": 1e308, "seed": -1, "scale": "inf"}}),
        json!({"kind": "copyToShape", "props": {"target": 1e308, "mode": "populate", "count": 1000}}),
        json!({"kind": "copyToShape", "props": {"target": "x".repeat(10_000), "align": "x"}, "modifier": "m1", "index": 0}),
        json!({"kind": "copyToShape", "props": {"target": -3, "mode": "points"}, "id": -1}),
        json!({"kind": "stencil", "props": {"threshold": 1e308, "widthMm": -1e308, "marginMm": "inf", "toleranceMm": "NaN", "minCutAreaMm2": 1e308}}),
        json!({"kind": "stencil", "props": {"colourMode": 7, "preview": [1], "paperColour": "\u{0}", "cutColours": ["#zzz", 5, null, "x".repeat(10_000)]}}),
        json!({"kind": "stencil", "props": {"cutColours": vec!["#abcdef"; 5_000], "invert": "x", "clothColour": {"a": 1}}, "index": 0}),
        json!({"upTo": 1e308, "index": 0}),
        json!({"upTo": 0}),
        json!({"upTo": -1, "ids": []}),
    ];
    let ids: Vec<&str> = crate::cmd::command_specs().iter().map(|c| c.id).filter(|i| i.starts_with("modifier.")).collect();
    assert!(ids.len() >= 9, "{ids:?}");
    for cmd in ids {
        for j in &junk {
            for damaged in [false, true] {
                let (mut s, r) = arrayed();
                let _ = s.execute("modifier.copy", &json!({"id": r.0}));
                if damaged {
                    put(&mut s, r, "{\"stack\": [1]}");
                }
                let mut p = j.clone();
                if p.get("ids").is_none() && p.get("id").is_none() {
                    p["ids"] = json!([r.0]);
                }
                // Junk keys the description doesn't name are refused before the command runs;
                // run the command itself with them too.
                let spec = crate::cmd::find_command(cmd).unwrap();
                for r in [s.execute(cmd, &p), s.run_nested(|s| (spec.run)(s, &p))] {
                    if let Err(EngineError::Internal { cmd, msg }) = r {
                        panic!("{cmd} panicked on {p}: {msg}");
                    }
                }
            }
        }
    }
}
