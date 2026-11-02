//! PICABIA: `param.*`, `modifier.bind` and `modifier.unbind`: parameters and bindings through the
//! commands, undo, save and reopen, SVG baking, Apply, data sets, row moves and object copies.

use serde_json::{Value, json};
use vectorcraft_doc::{DataSet, DataValue, NodeId, Variable, VariableKind};

use super::tests::{err, ex, id_of, rect, session, undo_labels};
use crate::Session;

fn instances(s: &mut Session, id: NodeId) -> u64 {
    ex(s, "modifier.list", json!({"id": id.0}))["instances"].as_u64().unwrap()
}

/// A rectangle with a linear array whose count is bound to `copies` (3).
fn bound() -> (Session, NodeId) {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    ex(&mut s, "param.add", json!({"name": "copies", "value": 3, "min": 1, "max": 20}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"count": 2, "offsetX": 120}}));
    ex(&mut s, "modifier.bind", json!({"ids": [r.0], "index": 0, "field": "count", "binding": {"param": "copies"}}));
    (s, r)
}

/// PIC6.3a: a slider drag (begin, previews, commit) is one undo step; setting what is already
/// there is none.
#[test]
fn a_drag_is_one_undo_step_and_an_unchanged_set_is_none() {
    let (mut s, r) = bound();
    let steps = undo_labels(&s).len();
    s.begin_interaction("Set Parameter").unwrap();
    for v in [4, 6, 9, 7] {
        s.preview("param.set", &json!({"name": "copies", "value": v})).unwrap();
    }
    s.commit_interaction().unwrap();
    assert_eq!(undo_labels(&s).len(), steps + 1);
    assert_eq!(instances(&mut s, r), 7);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(instances(&mut s, r), 3, "one undo goes back past the whole drag");
    ex(&mut s, "param.set", json!({"name": "copies", "value": 3}));
    ex(&mut s, "param.set", json!({"name": "copies"}));
    assert_eq!(undo_labels(&s).len(), steps, "no change, no step");
}

/// PIC6.3a: stacks inside a symbol's art read the document's parameters: a rename rewrites them
/// and a removal reports them; a rename onto a name dangling bindings read warns.
#[test]
fn symbol_art_follows_renames_and_dangling_names_warn() {
    let (mut s, r) = bound();
    ex(&mut s, "select.set", json!({"ids": [r.0]}));
    ex(&mut s, "symbol.new", json!({"name": "Mark"}));
    let doc = &s.doc().unwrap().doc;
    let art_id = doc.symbols.iter().find(|x| x.name == "Mark").unwrap().art.id;
    assert_eq!(super::params::objects_reading(doc, "copies").len(), 1, "the art's object reads it");
    ex(&mut s, "param.set", json!({"name": "copies", "rename": "count"}));
    let doc = &s.doc().unwrap().doc;
    let mut src = vec![];
    doc.symbols.iter().find(|x| x.name == "Mark").unwrap().art.walk(&mut |n| src.extend(vectorcraft_modifiers::node_source(n).map(str::to_string)));
    assert!(src.iter().any(|x| x.contains(r#""param":"count""#)), "{src:?} (art {art_id:?})");
    let removed = ex(&mut s, "param.remove", json!({"name": "count"}));
    assert_eq!(removed["stillBound"].as_array().unwrap().len(), 1, "{removed}");
    // A new parameter renamed onto the name the art still reads warns that it now drives it.
    ex(&mut s, "param.add", json!({"name": "other", "value": 2}));
    let set = ex(&mut s, "param.set", json!({"name": "other", "rename": "count"}));
    assert!(set["warning"].as_str().is_some_and(|w| w.contains("already read `count`")), "{set}");
}

#[test]
fn parameters_add_set_remove_list_with_undo() {
    let mut s = session();
    let p = ex(&mut s, "param.add", json!({"name": "gap", "value": 12, "min": 0, "max": 50, "step": 0.5}));
    assert_eq!(p["parameter"], json!({"name": "gap", "type": "number", "value": 12.0, "min": 0.0, "max": 50.0, "step": 0.5}));
    ex(&mut s, "param.add", json!({"name": "label", "type": "text", "value": "hi"}));
    assert_eq!(undo_labels(&s).last().unwrap(), "Add Parameter");
    assert!(err(&mut s, "param.add", json!({"name": "GAP"})).contains("already"));
    assert!(err(&mut s, "param.add", json!({"name": "sin"})).contains("means something"));
    assert!(
        err(&mut s, "param.add", json!({"name": "x", "value": 5, "max": 2})).contains("more than")
            || err(&mut s, "param.add", json!({"name": "x", "value": 5, "max": 2})).contains("outside")
    );
    ex(&mut s, "param.set", json!({"name": "gap", "value": 20}));
    assert!(err(&mut s, "param.set", json!({"name": "gap", "value": 99})).contains("outside"));
    assert!(err(&mut s, "param.set", json!({"name": "nope", "value": 1})).contains("no parameter"));
    let l = ex(&mut s, "param.list", json!({}));
    assert_eq!(l["parameters"][0]["effective"], json!(20.0));
    assert_eq!(l["parameters"][1]["effective"], json!("hi"));
    assert_eq!(l["dataSet"], Value::Null);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "param.list", json!({}))["parameters"][0]["value"], json!(12.0));
    ex(&mut s, "param.remove", json!({"name": "label"}));
    assert_eq!(ex(&mut s, "param.list", json!({}))["parameters"].as_array().unwrap().len(), 1);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "param.list", json!({}))["parameters"].as_array().unwrap().len(), 2);
}

#[test]
fn a_bound_count_follows_its_parameter_and_undo() {
    let (mut s, r) = bound();
    assert_eq!(instances(&mut s, r), 3);
    assert_eq!(undo_labels(&s).last().unwrap(), "Bind Modifier Field");
    ex(&mut s, "param.set", json!({"name": "copies", "value": 7}));
    assert_eq!(instances(&mut s, r), 7);
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["modifiers"][0]["bind"], json!({"count": {"param": "copies"}}));
    assert_eq!(l["bound"][0], json!({"modifier": "m1", "field": "count", "value": 7.0, "perCopy": false, "error": null}));
    assert_eq!(l["modifiers"][0]["count"], json!(2), "the typed value stays");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(instances(&mut s, r), 3);
    // Removing the parameter: the typed value, with a warning on the field; adding it back binds again.
    let rm = ex(&mut s, "param.remove", json!({"name": "copies"}));
    assert_eq!(rm["stillBound"], json!([r.0]));
    assert_eq!(instances(&mut s, r), 2);
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert!(l["bound"][0]["error"].as_str().unwrap().contains("no parameter"), "{l}");
    assert!(l["warnings"].as_array().unwrap().iter().any(|w| w["message"].as_str().unwrap().contains("copies")));
    ex(&mut s, "param.add", json!({"name": "copies", "value": 4}));
    assert_eq!(instances(&mut s, r), 4);
    // Unbind: the typed value; nothing bound is refused, with no undo step.
    ex(&mut s, "modifier.unbind", json!({"ids": [r.0], "index": 0, "field": "count"}));
    assert_eq!(instances(&mut s, r), 2);
    let steps = undo_labels(&s).len();
    assert!(s.execute("modifier.unbind", &json!({"ids": [r.0], "index": 0})).is_err());
    assert_eq!(undo_labels(&s).len(), steps);
}

#[test]
fn bind_refuses_what_it_cannot_drive() {
    let (mut s, r) = bound();
    for (p, why) in [
        (json!({"field": "align", "binding": {"param": "copies"}}), "isn't a number field"),
        (json!({"field": "count", "binding": {"param": "nope"}}), "no parameter"),
        (json!({"field": "count", "binding": {"expr": "copies +"}}), "ends too soon"),
        (json!({"field": "count", "binding": {"expr": "nope * 2"}}), "unknown name"),
        (json!({"field": "count", "binding": {"ramp": {}}}), "ramps"),
        (json!({"field": "count", "binding": "copies"}), "binding"),
        (json!({"field": "count"}), "binding"),
        (json!({"binding": {"param": "copies"}}), "field"),
    ] {
        let mut q = p.clone();
        q["ids"] = json!([r.0]);
        q["index"] = json!(0);
        let e = err(&mut s, "modifier.bind", q);
        assert!(e.contains(why), "{p}: {e}");
    }
}

#[test]
fn per_copy_expressions_and_rename() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    ex(&mut s, "param.add", json!({"name": "step", "value": 10, "max": 90}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "linearArray", "props": {"count": 4, "offsetX": 150}}));
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "transformEach"}));
    let b = ex(&mut s, "modifier.bind", json!({"ids": [r.0], "index": 1, "field": "rotate", "binding": {"expr": "i * step + Step * 0"}}));
    assert_eq!(b["perCopy"], json!(true));
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["bound"][0]["perCopy"], json!(true));
    // Renaming the parameter rewrites the expression (every case of the name, not functions).
    ex(&mut s, "param.set", json!({"name": "step", "rename": "angle"}));
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["modifiers"][1]["bind"]["rotate"], json!({"expr": "i * angle + angle * 0"}));
    assert!(l["bound"][0]["error"].is_null(), "{l}");
    assert!(err(&mut s, "param.set", json!({"name": "angle", "rename": "i"})).contains("means something"));
    // One undo step for the rename and the rewrite.
    ex(&mut s, "edit.undo", json!({}));
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["modifiers"][1]["bind"]["rotate"], json!({"expr": "i * step + Step * 0"}));
}

#[test]
fn bindings_follow_rows_through_moves_duplicates_and_object_copies() {
    let (mut s, r) = bound();
    ex(&mut s, "modifier.add", json!({"ids": [r.0], "kind": "mirror", "index": 0}));
    // Moved down a row: still bound by row id.
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": r.0}))["modifiers"][1]["bind"]["count"], json!({"param": "copies"}));
    ex(&mut s, "modifier.duplicate", json!({"ids": [r.0], "index": 1}));
    let l = ex(&mut s, "modifier.list", json!({"id": r.0}));
    assert_eq!(l["modifiers"][2]["bind"], l["modifiers"][1]["bind"], "the duplicate row is bound the same way");
    // Removing a row drops its bindings, not the other row's.
    ex(&mut s, "modifier.remove", json!({"ids": [r.0], "index": 2}));
    assert_eq!(ex(&mut s, "modifier.list", json!({"id": r.0}))["bound"].as_array().unwrap().len(), 1);
    // A duplicated object reads the same parameter.
    let dup = id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [r.0]}))["ids"][0]);
    ex(&mut s, "param.set", json!({"name": "copies", "value": 5}));
    assert_eq!(instances(&mut s, r), 10, "mirror × 5");
    assert_eq!(instances(&mut s, dup), 10);
    // Copied rows paste with their bindings.
    let other = rect(&mut s, 900.0);
    ex(&mut s, "modifier.copy", json!({"id": r.0, "index": 1}));
    ex(&mut s, "modifier.paste", json!({"ids": [other.0]}));
    assert_eq!(instances(&mut s, other), 5);
}

#[test]
fn parameters_and_bindings_survive_save_and_reopen() {
    let (mut s, r) = bound();
    ex(&mut s, "param.set", json!({"name": "copies", "value": 6}));
    let saved = ex(&mut s, "document.serialize", json!({"format": "vectorcraft"}));
    let mut t = Session::new();
    ex(&mut t, "document.open", json!({"name": "params.vectorcraft", "dataBase64": saved["dataBase64"]}));
    assert_eq!(ex(&mut t, "param.list", json!({}))["parameters"][0]["value"], json!(6.0));
    assert_eq!(instances(&mut t, r), 6);
}

#[test]
fn svg_export_and_apply_bake_the_bound_values() {
    let (mut s, r) = bound();
    ex(&mut s, "param.set", json!({"name": "copies", "value": 4}));
    let svg = ex(&mut s, "document.serialize", json!({"format": "svg", "preserveEditing": false}))["text"].as_str().unwrap().to_string();
    assert_eq!(svg.matches("<path").count() + svg.matches("<rect").count(), 4, "{svg}");
    let applied = ex(&mut s, "modifier.apply", json!({"ids": [r.0]}));
    assert_eq!(applied["applied"][0]["copies"], json!(4));
}

#[test]
fn a_data_set_drives_a_parameter() {
    let (mut s, r) = bound();
    ex(&mut s, "param.set", json!({"name": "copies", "source": "n_copies"}));
    s.edit("Data", |d, _| {
        let v = &mut *d.variables;
        v.variables.push(Variable { name: "n_copies".into(), kind: VariableKind::Text });
        for (name, val) in [("small", "2"), ("big", "9")] {
            v.datasets.push(DataSet { name: name.into(), values: [("n_copies".to_string(), DataValue::Text(val.into()))].into() });
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(instances(&mut s, r), 3, "no active data set: its own value");
    ex(&mut s, "dataset.select", json!({"name": "big"}));
    assert_eq!(instances(&mut s, r), 9);
    let l = ex(&mut s, "param.list", json!({}));
    assert_eq!(
        (l["dataSet"].clone(), l["parameters"][0]["effective"].clone(), l["parameters"][0]["fromDataSet"].clone()),
        (json!("big"), json!(9.0), json!(true))
    );
    ex(&mut s, "dataset.select", json!({"name": "small"}));
    assert_eq!(instances(&mut s, r), 2);
}

#[test]
fn parameter_commands_document_their_params() {
    for (id, names) in [
        ("param.add", vec!["name", "type", "value", "min", "max", "step", "source"]),
        ("param.set", vec!["name", "value", "min", "max", "step", "source", "rename"]),
        ("param.remove", vec!["name"]),
        ("param.list", vec![]),
        ("modifier.bind", vec!["ids", "id", "modifier", "index", "field", "binding"]),
        ("modifier.unbind", vec!["ids", "id", "modifier", "index", "field"]),
    ] {
        let spec = crate::cmd::find_command(id).unwrap();
        assert_eq!(crate::cmd::params::doc_keys(spec.params).unwrap().keys, names, "{id}");
    }
}

#[test]
fn parameter_and_binding_commands_survive_junk() {
    let junk = [
        json!({"name": null, "value": "x"}),
        json!({"name": "x".repeat(10_000), "value": 1e308}),
        json!({"name": "\u{0}", "type": 7, "min": "inf", "max": -1e308}),
        json!({"name": "copies", "value": [1], "source": {"a": 1}, "rename": 5}),
        json!({"name": "copies", "rename": "x".repeat(300)}),
        json!({"name": "copies", "step": -1, "min": 5, "max": 1}),
        json!({"field": "count", "binding": {"expr": "(".repeat(300)}, "index": 0}),
        json!({"field": "count", "binding": {"expr": "copies ^ copies ^ copies ^ 99"}, "index": 0}),
        json!({"field": "\u{0}", "binding": {"param": "\u{0}"}, "modifier": "m1"}),
        json!({"field": "count", "binding": {"param": "copies", "expr": "1"}, "index": 1e308}),
        json!({"field": [1], "binding": null, "ids": [{"x": 1}]}),
    ];
    for cmd in ["param.add", "param.set", "param.remove", "param.list", "modifier.bind", "modifier.unbind"] {
        for j in &junk {
            let (mut s, r) = bound();
            let _ = s.execute(cmd, j);
            assert!(s.execute("modifier.list", &json!({"id": r.0})).is_ok(), "{cmd} {j}");
            assert!(s.execute("param.list", &json!({})).is_ok(), "{cmd} {j}");
        }
    }
}
