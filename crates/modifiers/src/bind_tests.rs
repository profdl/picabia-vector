//! Parameters and field bindings: the binding expression mode, the parameter model and data sets,
//! resolution (per copy and once), failures, caps, storage and the evaluation cache key.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Appearance, DataSet, DataValue, Document, Node, NodeId, Variable, VariableKind};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use crate::expr::{Expr, MAX_EXPR_LEN, names_in, reserved};
use crate::params::{MAX_PARAMS, check_name};
use crate::*;

const B: Rect = Rect::new(0.0, 0.0, 20.0, 10.0);

fn scope(pairs: &[(&str, f64)]) -> Scope {
    Scope::from_values(pairs.iter().map(|(k, v)| (k.to_string(), ParamValue::Number(*v))))
}

fn stack(mods: Value) -> Stack {
    stack_from_value(&json!({"v": 1, "stack": mods})).unwrap()
}

fn props(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn origin(t: Affine) -> Point {
    t * Point::ZERO
}

fn angle(t: Affine) -> f64 {
    let c = t.as_coeffs();
    c[1].atan2(c[0]).to_degrees()
}

#[test]
fn binding_expressions_read_parameters_and_the_clock() {
    let s = scope(&[("gap", 12.0), ("Steps", 3.0)]);
    let e = |t: &str| Expr::parse_binding(t, &s).unwrap().eval(&expr::Env { i: 2.0, n: 5.0, t: 0.75, value: 10.0 });
    assert_eq!(e("gap * 2"), 24.0);
    assert_eq!(e("STEPS + steps"), 6.0, "names ignore case");
    assert_eq!(e("t"), 0.0, "t is the clock (0), not the per-copy ramp");
    assert_eq!(e("i * gap + n + value"), 39.0);
    assert!(Expr::parse_binding("i * 2", &s).unwrap().varies());
    assert!(!Expr::parse_binding("gap * t + value", &s).unwrap().varies(), "parameters, t and value don't vary per copy");
    let err = Expr::parse_binding("gapp + 1", &s).unwrap_err();
    assert!(err.message.contains("unknown name `gapp`") && err.message.contains("parameter"), "{err}");
    let texts = Scope::from_values([("label".to_string(), ParamValue::Text("abc".into())), ("num".to_string(), ParamValue::Text(" 4.5 ".into()))]);
    assert!(Expr::parse_binding("label", &texts).unwrap_err().message.contains("not a number"));
    assert_eq!(Expr::parse_binding("num * 2", &texts).unwrap().eval(&expr::Env::default()), 9.0, "a numeric text reads as a number");
    // The per-copy language keeps its `t` and refuses parameter names.
    assert!(Expr::parse("gap").is_err());
    assert_eq!(Expr::parse("t").unwrap().eval(&expr::Env::new(1, 3, 0.0)), 0.5);
}

#[test]
fn names_in_lists_parameters_only() {
    assert_eq!(names_in("gap * sin(i * Angle) + value + pi + t + n + gap"), vec!["gap", "angle"]);
    assert!(names_in("1 + ").is_empty());
    assert!(names_in(&"a".repeat(MAX_EXPR_LEN + 1)).is_empty());
    assert!(names_in("$").is_empty(), "text that doesn't lex reads nothing");
}

#[test]
fn renaming_rewrites_reads_of_the_name_only() {
    assert_eq!(expr::rename_in("Gap*2 + gapx + max(gap, 1)", "gap", "space"), "space*2 + gapx + max(space, 1)");
    assert_eq!(expr::rename_in("min(1) + i", "min", "x"), "min(1) + i", "a function call isn't the name");
    assert_eq!(expr::rename_in("gap $", "gap", "x"), "gap $", "text that doesn't lex is left alone");
    assert_eq!(Binding::Param("Gap".into()).renamed("gap", "space"), Some(Binding::Param("space".into())));
    assert_eq!(Binding::Expr("i * 2".into()).renamed("gap", "space"), None);
}

#[test]
fn parameter_names_are_checked() {
    for ok in ["copies", "_x", "Gap2", "a_b_c"] {
        assert!(check_name(ok).is_ok(), "{ok}");
    }
    for bad in ["", "2x", "a-b", "a b", "é", "i", "N", "t", "value", "PI", "sin", "rand", &"x".repeat(33)] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    assert!(reserved("Clamp") && !reserved("gap"));
    let mut ps = Parameters::default();
    ps.list.push(Parameter::new("Gap", &Map::new()).unwrap());
    assert!(ps.check_new_name("gap", None).is_err(), "unique ignoring case");
    assert!(ps.check_new_name("GAP", Some("gap")).is_ok(), "renaming to itself in another case");
}

#[test]
fn parameters_validate_and_clamp() {
    let p = Parameter::new("n2", &props(json!({"value": 500}))).unwrap();
    assert_eq!(p.kind, ParamKind::Number { value: 500.0, min: 0.0, max: 500.0, step: 1.0 }, "the default range widens to take the value");
    let mut p = Parameter::new("k", &props(json!({"value": 5, "min": 1, "max": 10, "step": 0.5}))).unwrap();
    assert!(p.set(&props(json!({"value": 11}))).is_err(), "a value given outside the range is refused");
    assert!(p.set(&props(json!({"min": 20}))).is_err(), "min over max");
    assert!(p.set(&props(json!({"step": -1}))).is_err());
    assert!(p.set(&props(json!({"value": f64::MAX}))).is_err());
    assert!(p.set(&props(json!({"value": "x"}))).is_err());
    assert!(p.set(&props(json!({"colour": 1}))).is_err(), "unknown keys");
    p.set(&props(json!({"max": 3}))).unwrap();
    assert_eq!(p.own_value(), ParamValue::Number(3.0), "a moved range clamps the value");
    let before = p.clone();
    assert!(p.set(&props(json!({"max": 4, "min": "junk"}))).is_err());
    assert_eq!(p, before, "a refused change leaves it unchanged");
    let t = Parameter::new("label", &props(json!({"type": "text", "value": "x".repeat(5000)}))).unwrap();
    assert!(matches!(&t.kind, ParamKind::Text { value } if value.len() == params::MAX_TEXT_LEN));
    assert!(Parameter::new("label", &props(json!({"type": "text", "min": 1}))).is_err());
    assert!(Parameter::new("x", &props(json!({"type": "colour"}))).is_err());
}

#[test]
fn stored_parameters_read_leniently_and_round_trip() {
    let stored = json!({"v": 1, "params": [
        {"name": "a", "type": "number", "value": 5, "min": 10, "max": 0, "step": -3},
        {"name": "A", "value": 1},
        {"name": "2bad"},
        "junk",
        {"name": "label", "type": "text", "value": "hi", "source": "Title"},
        {"name": "huge", "value": 1e300}
    ]});
    let (ps, dropped) = Parameters::from_value(Some(&stored));
    assert_eq!(dropped, 3);
    assert_eq!(ps.list.len(), 3);
    assert_eq!(ps.get("a").unwrap().kind, ParamKind::Number { value: 5.0, min: 0.0, max: 10.0, step: 0.0 });
    assert_eq!(ps.get("huge").unwrap().own_value(), ParamValue::Number(0.0), "junk numbers take their defaults");
    assert_eq!(ps.get("LABEL").unwrap().source.as_deref(), Some("Title"));
    let again = Parameters::from_value(Some(&ps.to_value())).0;
    assert_eq!(again, ps);
    let many = json!({"params": (0..MAX_PARAMS + 10).map(|k| json!({"name": format!("p{k}")})).collect::<Vec<_>>()});
    assert_eq!(Parameters::from_value(Some(&many)).0.list.len(), MAX_PARAMS);
    assert_eq!(Parameters::from_value(Some(&json!(5))).0, Parameters::default());
}

fn doc_with_rect() -> (Document, NodeId) {
    let mut d = Document::new(612.0, 792.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(B), Appearance::default_art())).unwrap();
    (d, id)
}

fn set_params(d: &mut Document, list: Value) {
    let (ps, _) = Parameters::from_value(Some(&json!({"params": list})));
    ps.store(d);
}

fn set_stack(d: &mut Document, id: NodeId, mods: Value) {
    set_node_source(d.node_mut(id).unwrap(), Some(json!({"v": 1, "stack": mods}).to_string()));
}

#[test]
fn data_sets_supply_parameter_values() {
    let (mut d, _) = doc_with_rect();
    set_params(
        &mut d,
        json!([
            {"name": "copies", "value": 3, "min": 1, "max": 10, "source": "copies"},
            {"name": "label", "type": "text", "value": "own", "source": "label"},
            {"name": "gone", "value": 2, "source": "nope"}
        ]),
    );
    let v = &mut *d.variables;
    v.variables.push(Variable { name: "copies".into(), kind: VariableKind::Text });
    v.variables.push(Variable { name: "label".into(), kind: VariableKind::Text });
    let row = |c: &str, l: &str| DataSet {
        name: format!("row {c}"),
        values: BTreeMap::from([("copies".to_string(), DataValue::Text(c.into())), ("label".to_string(), DataValue::Text(l.into()))]),
    };
    v.datasets = vec![row("7", "seven"), row("99", "big"), row("abc", "x")];
    // No active data set: own values.
    let s = Scope::of_doc(&d);
    assert_eq!(s.get("copies").unwrap().value, ParamValue::Number(3.0));
    assert!(s.get("gone").unwrap().warning.as_deref().unwrap().contains("no variable"));
    let pick = |d: &mut Document, name: &str| d.variables.active_dataset = Some(name.into());
    pick(&mut d, "row 7");
    let s = Scope::of_doc(&d);
    assert_eq!(s.get("copies").unwrap().value, ParamValue::Number(7.0));
    assert!(s.get("copies").unwrap().from_dataset);
    assert_eq!(s.get("label").unwrap().value, ParamValue::Text("seven".into()));
    pick(&mut d, "row 99");
    assert_eq!(Scope::of_doc(&d).get("copies").unwrap().value, ParamValue::Number(10.0), "clamped to the range");
    pick(&mut d, "row abc");
    let s = Scope::of_doc(&d);
    assert_eq!(s.get("copies").unwrap().value, ParamValue::Number(3.0), "not a number: its own value");
    assert!(s.get("copies").unwrap().warning.is_some());
}

#[test]
fn a_bound_count_reads_its_parameter_clamped_and_whole() {
    let st = stack(json!([{"id": "m1", "kind": "linearArray", "count": 2, "bind": {"count": {"param": "copies"}}}]));
    let out = evaluate_in(&st, B, &NoTargets, &scope(&[("copies", 5.0)]));
    assert_eq!(out.instances.len(), 5);
    assert_eq!(out.bound, vec![BoundField { modifier: "m1".into(), field: "count".into(), value: Some(5.0), per_copy: false, error: None }]);
    assert_eq!(evaluate_in(&st, B, &NoTargets, &scope(&[("copies", 4.6)])).instances.len(), 5, "rounded");
    assert_eq!(evaluate_in(&st, B, &NoTargets, &scope(&[("copies", 1e9)])).instances.len(), MAX_COUNT as usize, "clamped to the field's range");
    assert_eq!(evaluate_in(&st, B, &NoTargets, &scope(&[("copies", -3.0)])).instances.len(), 1);
    // Without the parameter: the typed value, with a warning on the field.
    let out = evaluate_in(&st, B, &NoTargets, &Scope::default());
    assert_eq!(out.instances.len(), 2);
    let e = out.bound[0].error.as_deref().unwrap();
    assert!(e.contains("no parameter named `copies`"), "{e}");
    assert!(out.warnings.iter().any(|w| w.modifier.as_deref() == Some("m1") && w.message.contains("copies")));
    // The stack as written is untouched: bindings never write back.
    assert_eq!(st.modifiers[0].kind, ModifierKind::LinearArray(LinearArray { count: 2, ..LinearArray::default() }));
}

#[test]
fn once_evaluated_fields_see_the_copies_above_and_their_own_value() {
    let st = stack(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 30},
        {"id": "b", "kind": "linearArray", "count": 2, "offsetX": 0, "offsetY": 5, "bind": {"offsetY": {"expr": "n * 10 + value + i"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &Scope::default());
    assert_eq!(out.instances.len(), 6);
    let b = out.bound.iter().find(|b| b.modifier == "b").unwrap();
    assert_eq!(b.value, Some(35.0), "n = 3 copies above, value = 5, i = 0");
    assert_eq!(origin(out.instances[3].transform), Point::new(0.0, 35.0));
}

#[test]
fn per_copy_expressions_give_each_copy_its_value() {
    let st = stack(json!([
        {"id": "a", "kind": "linearArray", "count": 4, "offsetX": 0},
        {"id": "e", "kind": "transformEach", "progressive": true, "rotate": 90, "bind": {"rotate": {"expr": "i * step"}, "offsetY": {"param": "drop"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &scope(&[("step", 15.0), ("drop", 8.0)]));
    let rot: Vec<f64> = out.instances.iter().map(|i| (angle(i.transform) * 1e6).round() / 1e6).collect();
    assert_eq!(rot, vec![0.0, 15.0, 30.0, 45.0], "the binding replaces the progressive ramp");
    let e = out.bound.iter().find(|b| b.field == "rotate").unwrap();
    assert!(e.per_copy && e.value.is_none() && e.error.is_none());
    // A constant on a per-copy field is one value, ramped like the field's own.
    let d = out.bound.iter().find(|b| b.field == "offsetY").unwrap();
    assert_eq!((d.value, d.per_copy), (Some(8.0), false));
    // Determinism.
    assert_eq!(out, evaluate_in(&st, B, &NoTargets, &scope(&[("step", 15.0), ("drop", 8.0)])));
}

#[test]
fn a_binding_overrides_the_fields_own_per_copy_expression() {
    let st = stack(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 0},
        {"id": "e", "kind": "transformEach", "rotateExpr": "i * 100", "bind": {"rotate": {"expr": "i * 10"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &Scope::default());
    assert!((angle(out.instances[2].transform) - 20.0).abs() < 1e-9);
}

#[test]
fn randomize_amounts_bind_per_copy() {
    let st = stack(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 0},
        {"id": "r", "kind": "randomize", "position": 50, "rotation": 0, "scale": 0, "bind": {"position": {"expr": "i * 0"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &Scope::default());
    assert!(out.instances.iter().all(|i| origin(i.transform) == Point::ZERO), "position 0 for every copy");
}

#[test]
fn failures_fall_back_to_the_typed_value_with_a_warning() {
    for (bind, why) in [
        (json!({"offsetX": {"expr": "1 / 0"}}), "no number"),
        (json!({"offsetX": {"expr": "sqrt(-1)"}}), "no number"),
        (json!({"offsetX": {"expr": "nope * 2"}}), "unknown name"),
        (json!({"offsetX": {"expr": "2 +"}}), "ends too soon"),
        (json!({"offsetX": {"param": "label"}}), "not a number"),
        (json!({"offsetX": {"ramp": {"from": 0, "to": 1}}}), "isn't available yet"),
        (json!({"align": {"param": "x"}}), "isn't a number field"),
    ] {
        let st = stack(json!([{"id": "m", "kind": "linearArray", "count": 2, "offsetX": 40, "bind": bind}]));
        let texts = Scope::from_values([("label".to_string(), ParamValue::Text("abc".into()))]);
        let out = evaluate_in(&st, B, &NoTargets, &texts);
        assert_eq!(origin(out.instances[1].transform), Point::new(40.0, 0.0), "{bind}");
        let e = out.bound[0].error.as_deref().unwrap_or_default();
        assert!(e.contains(why), "{bind}: {e}");
    }
    // Per copy: the copies with no number use the typed value, and the count is reported.
    let st = stack(json!([
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 0},
        {"id": "e", "kind": "transformEach", "progressive": false, "rotate": 7, "bind": {"rotate": {"expr": "1 / (i - 1)"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &Scope::default());
    assert!((angle(out.instances[1].transform) - 7.0).abs() < 1e-9, "1/0 at copy 1: the typed value");
    let e = out.bound.iter().find(|b| b.field == "rotate").unwrap().error.as_deref().unwrap();
    assert!(e.contains("1 of 3 copies"), "{e}");
}

#[test]
fn geometry_and_stencil_rows_bind_too() {
    let st = stack(json!([
        {"id": "t", "kind": "trim", "start": 0, "end": 1, "bind": {"end": {"param": "e"}}},
        {"id": "s", "kind": "stencil", "bind": {"widthMm": {"expr": "w * 2"}, "threshold": {"param": "th"}}}
    ]));
    let out = evaluate_in(&st, B, &NoTargets, &scope(&[("e", 0.25), ("w", 60.0), ("th", 7.0)]));
    let st_row = out.stencil.as_ref().unwrap();
    assert_eq!(st_row.params.width_mm, 120.0);
    assert_eq!(st_row.params.threshold, 1.0, "clamped to the field's 0–1");
    assert!(matches!(out.geometry.first().map(|g| &g.op), Some(shape::GeometryOp::Trim(t)) if t.end == 0.25), "{:?}", out.geometry);
}

#[test]
fn bindings_round_trip_and_follow_their_row() {
    let src = json!({"v": 1, "stack": [
        {"id": "m1", "kind": "linearArray", "bind": {"count": {"param": "copies"}, "offsetX": {"ramp": [1, 2]}}},
        {"id": "m2", "kind": "mirror", "bind": "junk"}
    ]})
    .to_string();
    let st = parse(&src).unwrap();
    assert_eq!(st.modifiers[0].bind.len(), 2);
    assert!(matches!(st.modifiers[0].bind.get("offsetX"), Some(Binding::Other(_))));
    assert_eq!(st.modifiers[1].extra.get("bind"), Some(&json!("junk")), "a non-object `bind` is kept as it was");
    let again = parse(&to_json(&st)).unwrap();
    assert_eq!(again, st);
    // Reorder and duplicate: the bindings are the row's.
    let mut moved = st.clone();
    moved.modifiers.reverse();
    assert_eq!(moved.modifiers[1].bind.get("count"), Some(&Binding::Param("copies".into())));
    let dup = moved.modifiers[1].clone();
    moved.insert(2, dup).unwrap();
    assert_eq!(moved.modifiers[2].bind, moved.modifiers[1].bind);
    assert_ne!(moved.modifiers[2].id, moved.modifiers[1].id);
    // Removing the row drops them.
    moved.modifiers.remove(1);
    moved.modifiers.remove(1);
    assert!(!moved.has_bindings());
    // A row with more than MAX_BINDINGS keeps that many.
    let many: Map<String, Value> = (0..100).map(|k| (format!("f{k}"), json!({"param": "x"}))).collect();
    let st = stack(json!([{"kind": "mirror", "bind": many}]));
    assert_eq!(st.modifiers[0].bind.len(), MAX_BINDINGS);
}

#[test]
fn bind_field_validates() {
    let s = scope(&[("copies", 4.0)]);
    let mut m = Modifier::new("m1", "transformEach").unwrap();
    assert_eq!(m.bind_field("rotate", Binding::Expr("i * copies".into()), &s), Ok(true));
    assert_eq!(m.bind_field("jitter", Binding::Expr("i * copies".into()), &s), Ok(false), "jitter is read once");
    assert_eq!(m.bind_field("offsetX", Binding::Param("COPIES".into()), &s), Ok(false));
    assert!(m.bind_field("progressive", Binding::Param("copies".into()), &s).is_err(), "not a number field");
    assert!(m.bind_field("rotate", Binding::Param("nope".into()), &s).is_err());
    assert!(m.bind_field("rotate", Binding::Expr("1 +".into()), &s).is_err());
    assert!(m.bind_field("rotate", Binding::Expr("nope".into()), &s).is_err());
    assert!(m.bind_field("rotate", Binding::Other(json!({"ramp": 1})), &s).is_err());
    assert_eq!(m.bind.len(), 3);
    assert_eq!(m.unbind_field(Some("rotate")), 1);
    assert_eq!(m.unbind_field(Some("rotate")), 0);
    assert_eq!(m.unbind_field(None), 2);
    assert!(m.set_props(&props(json!({"bind": {}}))).is_err(), "bindings aren't props");
    for bad in [json!(1), json!({}), json!({"param": ""}), json!({"param": "a", "expr": "b"}), json!({"expr": "  "}), json!({"ramp": {}})] {
        assert!(Binding::from_request(&bad).is_err(), "{bad}");
    }
    assert_eq!(Binding::from_request(&json!({"expr": " i * 2 "})), Ok(Binding::Expr("i * 2".into())));
    assert!(Binding::from_request(&json!({"expr": "1".repeat(MAX_EXPR_LEN + 1)})).is_err());
}

#[test]
fn the_cache_key_holds_exactly_the_parameters_a_stack_reads() {
    let (mut d, id) = doc_with_rect();
    set_params(&mut d, json!([{"name": "copies", "value": 3, "max": 20}, {"name": "other", "value": 1}]));
    set_stack(&mut d, id, json!([{"id": "m1", "kind": "linearArray", "bind": {"count": {"param": "copies"}}}]));
    let a = evaluate_node_in_doc(&d, d.node(id).unwrap()).unwrap();
    assert_eq!(a.instances.len(), 3);
    let k = bound_key(&d, d.node(id).unwrap());
    assert!(has_bindings(d.node(id).unwrap()));
    // Another parameter changes: same evaluation, same key.
    set_params(&mut d, json!([{"name": "copies", "value": 3, "max": 20}, {"name": "other", "value": 9}]));
    let b = evaluate_node_in_doc(&d, d.node(id).unwrap()).unwrap();
    assert!(Arc::ptr_eq(&a, &b), "an unread parameter doesn't re-evaluate");
    assert_eq!(bound_key(&d, d.node(id).unwrap()), k);
    // The read one changes: a new evaluation and key.
    set_params(&mut d, json!([{"name": "copies", "value": 7, "max": 20}, {"name": "other", "value": 9}]));
    let c = evaluate_node_in_doc(&d, d.node(id).unwrap()).unwrap();
    assert_eq!(c.instances.len(), 7);
    assert_ne!(bound_key(&d, d.node(id).unwrap()), k);
    // An object without bindings reads no parameters.
    set_stack(&mut d, id, json!([{"id": "m1", "kind": "linearArray"}]));
    assert_eq!(bound_key(&d, d.node(id).unwrap()), 0);
    assert!(!has_bindings(d.node(id).unwrap()));
}

#[test]
fn junk_bindings_and_expressions_never_panic() {
    let junk = [
        json!(null),
        json!(1e308),
        json!("x"),
        json!({"param": 5}),
        json!({"expr": "((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))"}),
        json!({"expr": "9^9^9^9"}),
        json!({"expr": "rand(i) * 1e308 * 1e308"}),
        json!({"expr": "-".repeat(300)}),
        json!({"expr": "max(1,2,3,4,5,6,7,8,9)"}),
        json!({"param": "é"}),
    ];
    for kind in KINDS.iter().map(|(k, _)| *k) {
        let fields: Vec<&str> = number_fields(kind).iter().map(|f| f.key).chain(["nope", "count"]).collect();
        for f in &fields {
            for j in &junk {
                let st = stack(json!([{"id": "a", "kind": "linearArray", "count": 2}, {"id": "m", "kind": kind, "bind": {*f: j}}]));
                let out = evaluate_in(&st, B, &NoTargets, &scope(&[("x", f64::MAX)]));
                assert!(out.instances.iter().all(|i| i.transform.is_finite()), "{kind}.{f} = {j}");
                assert!(out.instances.len() <= MAX_INSTANCES_PER_NODE);
            }
        }
    }
}

/// PIC6.3a: every bound object of a frame shares one scope; a parameter edit builds one more.
#[test]
fn one_scope_per_document_version() {
    let (mut d, _) = doc_with_rect();
    set_params(&mut d, json!([{"name": "copies", "value": 3, "max": 20}]));
    let layer = d.layers[0].id;
    let mut ids = vec![];
    for k in 0..200 {
        let id = d.alloc_id();
        let r = Rect::new(f64::from(k), 0.0, f64::from(k) + 5.0, 5.0);
        d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(r), Appearance::default_art())).unwrap();
        set_stack(&mut d, id, json!([{"id": "m1", "kind": "linearArray", "bind": {"count": {"param": "copies"}}}]));
        ids.push(id);
    }
    let frame = |d: &Document, want: usize| {
        for id in &ids {
            assert_eq!(evaluate_node_in_doc(d, d.node(*id).unwrap()).unwrap().instances.len(), want);
            let _ = bound_key(d, d.node(*id).unwrap());
        }
    };
    let before = params::scope_builds();
    frame(&d, 3);
    frame(&d, 3);
    assert_eq!(params::scope_builds() - before, 1, "one scope for two frames of 200 bound objects");
    set_params(&mut d, json!([{"name": "copies", "value": 4, "max": 20}]));
    frame(&d, 4);
    assert_eq!(params::scope_builds() - before, 2, "a parameter edit builds one more");
}
