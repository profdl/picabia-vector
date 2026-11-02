//! PIC31.1: soft settings: presets and overrides, pins, lenient parsing, round trips.

use serde_json::json;

use crate::settings::BodyType;
use crate::soft::{MAX_PINS, Pin, Preset, Soft};

#[test]
fn soft_body_type_and_preset_round_trip() {
    let (s, w) =
        crate::parse(r#"{"bodyType":"soft","soft":{"preset":"cloth","stiffness":0.6,"pins":[{"u":0,"v":0.5},{"u":1,"v":0.5,"attachTo":7}]}}"#)
            .unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(s.body_type, BodyType::Soft);
    let soft = s.soft_body().unwrap();
    assert_eq!(soft.preset, Preset::Cloth);
    assert_eq!(soft.stiffness, Some(0.6));
    assert_eq!(soft.pins, vec![Pin { u: 0.0, v: 0.5, attach_to: None }, Pin { u: 1.0, v: 0.5, attach_to: Some(7) }]);
    // Only the overrides are stored, so a later preset tuning reaches the rest.
    let stored = s.to_value();
    assert_eq!(stored["soft"], json!({"preset": "cloth", "stiffness": 0.6, "pins": [{"u": 0.0, "v": 0.5}, {"u": 1.0, "v": 0.5, "attachTo": 7}]}));
    let (again, _) = crate::parse(&s.to_json()).unwrap();
    assert_eq!(again, s);
    // The fields: the override, the preset's for the rest.
    let f = soft.fields();
    assert_eq!(f.stiffness, 0.6);
    assert_eq!(f.bend, Preset::Cloth.defaults().bend);
    assert!(!f.self_collide, "cloth doesn't self-collide by default (PIC31.0's cost)");
}

#[test]
fn soft_without_settings_is_jelly_and_rigid_keeps_the_tuning() {
    let (s, _) = crate::parse(r#"{"bodyType":"soft"}"#).unwrap();
    assert_eq!(s.soft_body().unwrap(), Soft::of(Preset::Jelly));
    let (r, _) = crate::parse(r#"{"bodyType":"dynamic","soft":{"preset":"rubber"}}"#).unwrap();
    assert!(r.soft_body().is_none(), "a rigid body has no soft settings in effect");
    assert_eq!(r.soft.as_ref().unwrap().preset, Preset::Rubber, "but keeps them for switching back");
}

#[test]
fn junk_soft_settings_clamp_and_warn_and_keep_unknown_fields() {
    let (s, w) = crate::parse(
        r#"{"bodyType":"soft","soft":{"preset":"clay","stiffness":7,"bend":"x","damping":-1,"resolution":0.1,"selfCollide":2,"pins":[{"u":9,"v":0},{"u":"a"},{"u":0,"v":0,"attachTo":"x"}],"later":[1]}}"#,
    )
    .unwrap();
    let soft = s.soft.clone().unwrap();
    assert_eq!(soft.preset, Preset::Other("clay".into()));
    assert_eq!((soft.stiffness, soft.bend, soft.damping, soft.resolution, soft.self_collide), (Some(1.0), None, Some(0.0), Some(2.0), None));
    assert_eq!(soft.pins, vec![Pin { u: 4.0, v: 0.0, attach_to: None }, Pin { u: 0.0, v: 0.0, attach_to: None }]);
    assert!(w.len() >= 8, "{w:?}");
    assert_eq!(soft.extra.get("later"), Some(&json!([1])));
    let (again, _) = crate::parse(&s.to_json()).unwrap();
    assert_eq!(again, s);
    // An unknown preset simulates as jelly.
    assert_eq!(soft.preset.defaults(), Preset::Jelly.defaults());
    // `soft` not an object, `soft: null`.
    let (bad, w) = crate::parse(r#"{"bodyType":"soft","soft":3}"#).unwrap();
    assert!(bad.soft.is_none() && !w.is_empty());
    let mut s2 = s.clone();
    let w = s2.apply(json!({"soft": null}).as_object().unwrap(), false);
    assert!(w.is_empty() && s2.soft.is_none());
}

#[test]
fn null_drops_an_override_and_pins_are_capped() {
    let mut s = Soft::of(Preset::Rubber);
    s.apply(json!({"stiffness": 0.2}).as_object().unwrap(), false);
    assert_eq!(s.fields().stiffness, 0.2);
    s.apply(json!({"stiffness": null}).as_object().unwrap(), false);
    assert_eq!(s.fields().stiffness, Preset::Rubber.defaults().stiffness);
    let many: Vec<_> = (0..MAX_PINS + 10).map(|i| json!({"u": i as f64 / 1000.0, "v": 0})).collect();
    let w = s.apply(json!({"pins": many}).as_object().unwrap(), false);
    assert_eq!(s.pins.len(), MAX_PINS);
    assert!(!w.is_empty());
    let w = s.apply(json!({"wobble": 1}).as_object().unwrap(), false);
    assert_eq!(w.len(), 1, "unknown keys from a command are refused");
}

#[test]
fn new_mesh_only_for_kind_spacing_self_contact_and_pins() {
    let a = Soft::of(Preset::Jelly);
    let mut b = a.clone();
    b.stiffness = Some(0.9);
    b.damping = Some(0.9);
    assert!(!a.needs_new_mesh(&b), "material only");
    b.preset = Preset::Rubber;
    assert!(!a.needs_new_mesh(&b), "jelly and rubber are the same kind of body");
    b.preset = Preset::Cloth;
    assert!(a.needs_new_mesh(&b));
    let mut c = a.clone();
    c.resolution = Some(30.0);
    assert!(a.needs_new_mesh(&c));
}

#[test]
fn params_map_into_rapier_ranges() {
    for p in [Preset::Cloth, Preset::Jelly, Preset::Rubber] {
        let q = p.defaults().params();
        assert!((1.0..=60.0).contains(&q.edge_hz) && (0.05..=1.0).contains(&q.damping_ratio), "{p:?} {q:?}");
    }
    let mut f = Preset::Jelly.defaults();
    f.shape_keep = 0.0;
    assert_eq!(f.params().shape_hz, 0.0, "no shape memory: shape matching off");
}
