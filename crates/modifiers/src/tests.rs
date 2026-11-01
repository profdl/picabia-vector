use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::*;

fn stack(v: Value) -> Stack {
    parse(&v.to_string()).unwrap()
}

fn linear(params: Value) -> Stack {
    let mut m = json!({"id": "m1", "kind": "linearArray"});
    m.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    stack(json!({"v": 1, "stack": [m]}))
}

const B: Rect = Rect::new(0.0, 0.0, 20.0, 10.0);

fn close(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn empty_and_disabled_stacks_draw_normally() {
    assert!(evaluate(&Stack::default(), B).instances.is_empty());
    let s = stack(json!({"v": 1, "stack": [{"id": "m1", "enabled": false, "kind": "linearArray", "count": 4}]}));
    assert!(evaluate(&s, B).instances.is_empty());
}

#[test]
fn linear_array_transforms() {
    let s = linear(json!({"count": 4, "offsetX": 30, "offsetY": -5}));
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), 4);
    for (k, i) in out.instances.iter().enumerate() {
        assert_eq!(i.index, k as u32);
        assert_eq!(i.stage_indices, vec![k as u32]);
        assert!(close(i.transform, Affine::translate(Vec2::new(30.0 * k as f64, -5.0 * k as f64))), "{k}: {:?}", i.transform);
    }
    assert!(close(out.instances[0].transform, Affine::IDENTITY), "copy 0 is the original");
    assert!(out.warnings.is_empty());
    assert!(out.guides.contains(&Guide::Line { from: Point::new(10.0, 5.0), to: Point::new(100.0, -10.0) }));
}

#[test]
fn linear_array_rotates_and_scales_around_the_pivot() {
    let s = linear(json!({"count": 3, "offsetX": 0, "rotateEach": 90, "scaleEach": 2}));
    let out = evaluate(&s, B);
    let pivot = B.center();
    let t2 = out.instances[2].transform;
    // The pivot stays put (no offset); a point right of it goes left after 180° and 4×.
    assert!((t2 * pivot - pivot).hypot() < 1e-9);
    let p = t2 * Point::new(pivot.x + 1.0, pivot.y);
    assert!((p - Point::new(pivot.x - 4.0, pivot.y)).hypot() < 1e-9, "{p:?}");
}

#[test]
fn stacked_arrays_repeat_the_whole_block() {
    let s = stack(json!({"stack": [
        {"id": "a", "kind": "linearArray", "count": 3, "offsetX": 10},
        {"id": "b", "kind": "linearArray", "count": 2, "offsetX": 0, "offsetY": 100},
    ]}));
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), 6);
    // new_index = k * n_prev + j
    let i4 = &out.instances[4];
    assert_eq!((i4.index, i4.stage_indices.clone()), (4, vec![1, 1]));
    assert!(close(i4.transform, Affine::translate((10.0, 100.0))));
}

#[test]
fn nan_inf_and_huge_params_are_clamped() {
    // JSON has no NaN: a "NaN" string doesn't read as a number, so the modifier is kept as-is and ignored.
    let s = linear(json!({"count": 3, "offsetX": "NaN"}));
    assert!(matches!(s.modifiers[0].kind, ModifierKind::Other { ref error, .. } if error.is_some()));
    let out = evaluate(&s, B);
    assert!(out.instances.is_empty() && !out.warnings.is_empty());
    assert!(to_json(&s).contains("\"NaN\""), "kept on a round trip");
    // Params set in code can be anything: evaluate clamps them.
    let mut s = linear(json!({}));
    s.modifiers[0].kind = ModifierKind::LinearArray(LinearArray {
        count: 5,
        offset_x: f64::NAN,
        offset_y: f64::INFINITY,
        rotate_each: f64::NEG_INFINITY,
        scale_each: f64::NAN,
    });
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), 5);
    assert!(out.instances.iter().all(|i| i.transform.is_finite()));
    assert!(out.warnings.len() >= 4);
    // Huge but finite values.
    let s = linear(json!({"count": 1000, "offsetX": 1e308, "rotateEach": 1e308, "scaleEach": 1e308}));
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), 1000);
    assert!(out.instances.iter().all(|i| i.transform.is_finite()));
    // Non-finite bounds.
    let out = evaluate(&linear(json!({"count": 2})), Rect::new(f64::NAN, 0.0, f64::INFINITY, 1.0));
    assert_eq!(out.instances.len(), 2);
    assert!(out.instances.iter().all(|i| i.transform.is_finite()));
}

#[test]
fn counts_zero_negative_fractional_and_huge() {
    for (count, want) in [(json!(0), 1), (json!(-7), 1), (json!(2.6), 3), (json!(1e12), 1000), (json!("4"), 4)] {
        let s = linear(json!({"count": count}));
        assert_eq!(evaluate(&s, B).instances.len(), want, "count {count}");
    }
}

#[test]
fn the_instance_product_is_capped_before_allocating() {
    let s = stack(json!({"stack": [
        {"id": "a", "kind": "linearArray", "count": 1000},
        {"id": "b", "kind": "linearArray", "count": 10},
        {"id": "c", "kind": "linearArray", "count": 2},
        {"id": "d", "kind": "linearArray", "count": 3},
    ]}));
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), MAX_INSTANCES_PER_NODE, "the stages that fit");
    assert!(out.warnings.iter().any(|w| w.modifier.as_deref() == Some("c")), "{:?}", out.warnings);
    // 64 arrays of 1000: never allocates 1000^64.
    let many: Vec<Value> = (0..64).map(|i| json!({"id": format!("m{i}"), "kind": "linearArray", "count": 1000})).collect();
    let out = evaluate(&stack(json!({"stack": many})), B);
    assert_eq!(out.instances.len(), 1000);
}

#[test]
fn invalid_json_and_shapes_are_refused() {
    for bad in ["", "{", "nope", "[1,2]", "{\"stack\": 5}", "{\"stack\": [1]}", "{\"v\": -1}", "{\"v\": \"x\"}", "null"] {
        assert!(parse(bad).is_err(), "{bad}");
    }
    assert!(matches!(parse("{"), Err(ModError::Json(_))));
    assert!(parse("{}").unwrap().is_empty());
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    assert!(parse(&deep).is_err());
}

#[test]
fn oversize_sources_are_refused() {
    let pad = "x".repeat(MAX_SOURCE_BYTES);
    let src = json!({"v": 1, "stack": [], "pad": pad}).to_string();
    assert!(matches!(parse(&src), Err(ModError::TooLarge { .. })));
    assert!(parse_cached(&src).is_err());
}

#[test]
fn stacks_over_64_are_refused() {
    let many = |n: usize| json!({"stack": (0..n).map(|_| json!({"kind": "linearArray"})).collect::<Vec<_>>()}).to_string();
    assert_eq!(parse(&many(MAX_STACK_LEN)).unwrap().len(), MAX_STACK_LEN);
    assert!(matches!(parse(&many(MAX_STACK_LEN + 1)), Err(ModError::TooManyModifiers { count: 65 })));
    let mut s = parse(&many(MAX_STACK_LEN)).unwrap();
    assert!(s.insert(0, Modifier::new("", "mirror").unwrap()).is_err());
}

#[test]
fn unknown_kinds_and_fields_round_trip() {
    let src = json!({"v": 1, "future": {"a": 1}, "stack": [
        {"id": "m1", "enabled": true, "kind": "linearArray", "count": 2, "offsetX": 5, "wobble": [1, 2]},
        {"id": "m2", "enabled": false, "kind": "spiralArray", "turns": 3, "nested": {"x": "y"}},
    ]});
    let s = stack(src.clone());
    assert_eq!(s.modifiers[0].extra.get("wobble"), Some(&json!([1, 2])));
    assert!(matches!(&s.modifiers[1].kind, ModifierKind::Other { kind, error: None } if kind == "spiralArray"));
    let back: Value = serde_json::from_str(&to_json(&s)).unwrap();
    assert_eq!(back["future"], src["future"]);
    assert_eq!(back["stack"][0]["wobble"], json!([1, 2]));
    assert_eq!(back["stack"][1], src["stack"][1]);
    assert_eq!(parse(&to_json(&s)).unwrap(), s, "a second round trip changes nothing");
    // Unknown kinds are ignored (with a warning) when evaluating.
    let s = stack(json!({"stack": [{"id": "x", "kind": "spiralArray"}, {"id": "y", "kind": "linearArray", "count": 2}]}));
    let out = evaluate(&s, B);
    assert_eq!(out.instances.len(), 2);
    assert_eq!(out.warnings[0].modifier.as_deref(), Some("x"));
}

#[test]
fn every_kind_has_defaults_that_round_trip() {
    for (kind, label) in KINDS {
        let m = Modifier::new("m1", kind).unwrap();
        assert_eq!(m.label(), *label);
        let back = Modifier::from_value(&m.to_value()).unwrap();
        assert_eq!(back, m, "{kind}");
    }
    assert!(Modifier::new("m1", "spiralArray").is_none());
}

#[test]
fn missing_and_repeated_ids_get_fresh_ones() {
    let s = stack(json!({"stack": [{"kind": "mirror"}, {"id": "m1", "kind": "mirror"}, {"id": "m1", "kind": "mirror"}]}));
    let ids: Vec<&str> = s.modifiers.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().collect::<std::collections::BTreeSet<_>>().len() == 3, "{ids:?}");
}

#[test]
fn set_props_validates() {
    let mut m = Modifier::new("m1", "linearArray").unwrap();
    m.set_props(json!({"count": 7, "offsetX": "12.5", "enabled": false}).as_object().unwrap()).unwrap();
    assert_eq!(m.kind, ModifierKind::LinearArray(LinearArray { count: 7, offset_x: 12.5, ..LinearArray::default() }));
    assert!(!m.enabled);
    let before = m.clone();
    for bad in [
        json!({"offsetX": "NaN"}),
        json!({"offsetX": "inf"}),
        json!({"offsetX": [1]}),
        json!({"bogus": 1}),
        json!({"kind": "mirror"}),
        json!({"enabled": "maybe"}),
    ] {
        assert!(m.set_props(bad.as_object().unwrap()).is_err(), "{bad}");
        assert_eq!(m, before, "unchanged after {bad}");
    }
    let mut mirror = Modifier::new("m2", "mirror").unwrap();
    assert!(mirror.set_props(json!({"axis": "diagonal"}).as_object().unwrap()).is_err());
    mirror.set_props(json!({"axis": "horizontal", "keepOriginal": false}).as_object().unwrap()).unwrap();
    assert!(matches!(mirror.kind, ModifierKind::Mirror(Mirror { axis: MirrorAxis::Horizontal, keep_original: false, .. })));
}

#[test]
fn node_entry_points_cache_and_report() {
    let rect = vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 0.0, 120.0, 10.0));
    let mut n = vectorcraft_doc::Node::path(vectorcraft_doc::NodeId(5), rect, vectorcraft_doc::Appearance::default_art());
    assert!(node_instances(&n).is_none() && node_stack(&n).is_none());
    let pen = vectorcraft_doc::ForeignAttribute {
        ns: "https://example.org/plotter".into(),
        prefix: "plotter".into(),
        name: "pen".into(),
        value: "2".into(),
    };
    n.edit_attrs(|a| a.foreign.push(pen.clone()));
    set_node_source(&mut n, Some(json!({"v": 1, "stack": [{"id": "m1", "kind": "linearArray", "count": 3}]}).to_string()));
    assert_eq!(node_bounds(&n), Rect::new(100.0, 0.0, 120.0, 10.0));
    let a = node_instances(&n).unwrap();
    let b = node_instances(&n).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b), "cached");
    assert_eq!(a.transforms().count(), 3);
    set_node_source(&mut n, Some("{broken".into()));
    assert!(node_instances(&n).is_none());
    assert!(matches!(node_stack(&n), Some(Err(ModError::Json(_)))));
    // Removing the stack leaves no attribute behind (and keeps other foreign attributes).
    set_node_source(&mut n, None);
    assert!(!has_modifiers(&n));
    assert_eq!(n.extras().attrs.as_ref().map(|a| a.foreign.clone()), Some(vec![pen.clone()]), "other foreign attributes are kept");
    n.edit_attrs(|a| a.foreign.clear());
    assert!(n.extras().attrs.is_none(), "nothing left, no attributes record");
}

#[test]
fn seeds_are_deterministic() {
    let out = evaluate(&linear(json!({"count": 3})), B);
    assert_eq!(out.instances[1].seed, splitmix64(1));
    assert_ne!(out.instances[1].seed, out.instances[2].seed);
    assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
}
