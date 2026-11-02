//! Tests for the `color` row: its model, ramp, order, jitter, expressions and bindings.

use serde_json::{Value, json};
use vectorcraft_color::oklch_shift::OklchShift;
use vectorcraft_geom::Rect;

use crate::*;

const B: Rect = Rect::new(0.0, 0.0, 20.0, 10.0);

fn stack(mods: Value) -> Stack {
    parse(&json!({"v": 1, "stack": mods}).to_string()).unwrap()
}

fn eval(mods: Value) -> Output {
    evaluate(&stack(mods), B)
}

fn hues(out: &Output) -> Vec<f64> {
    out.colours.iter().map(|c| (c[0].fill.hue * 1e9).round() / 1e9).collect()
}

#[test]
fn defaults_round_trip_and_read_leniently() {
    let st = stack(json!([{"id": "c", "kind": "color"}]));
    let ModifierKind::Colour(p) = &st.modifiers[0].kind else { panic!("{:?}", st.modifiers[0].kind) };
    assert_eq!(*p, Colour::default());
    assert_eq!((p.hue, p.chroma, p.lightness, p.seed, p.amount, p.progressive), (10.0, 10.0, 10.0, 42, 1.0, true));
    // Round trip: every field written, expressions only when set, unknown fields kept.
    let st = stack(json!([{"id": "c", "kind": "color", "hue": "25", "stroke": "own", "order": "sideways", "array": 2.6,
        "seed": -3, "hueExpr": "i * 3", "future": [1, 2]}]));
    let ModifierKind::Colour(p) = &st.modifiers[0].kind else { panic!() };
    assert_eq!((p.hue, p.stroke, p.order, p.array, p.seed), (25.0, ColourStroke::Own, ColourOrder::Copies, 3, 0));
    assert_eq!(p.hue_expr.as_deref(), Some("i * 3"));
    let v = st.modifiers[0].to_value();
    assert_eq!(v["future"], json!([1, 2]));
    assert!(v.get("chromaExpr").is_none());
    assert_eq!(v["strokeJitterLightness"], json!(0.0));
    let back = stack(json!([v]));
    assert_eq!(back.modifiers, st.modifiers);
    // A mistyped number: the row doesn't read (kept, ignored with a warning), as for every row.
    let out = eval(json!([{"id": "c", "kind": "color", "hue": {"a": 1}}]));
    assert!(out.instances.is_empty() && out.warnings.iter().any(|w| w.message.contains("don't read")));
}

#[test]
fn set_props_and_bindable_fields() {
    let mut m = Modifier::new("c", "color").unwrap();
    let props = json!({"hue": 50, "stroke": "none", "hueExpr": "t * 100"});
    m.set_props(props.as_object().unwrap()).unwrap();
    let ModifierKind::Colour(p) = &m.kind else { panic!() };
    assert_eq!((p.hue, p.stroke, p.hue_expr.as_deref()), (50.0, ColourStroke::None, Some("t * 100")));
    assert!(m.set_props(json!({"hueExpr": "i +"}).as_object().unwrap()).is_err());
    assert!(m.set_props(json!({"tint": 1}).as_object().unwrap()).is_err());
    for key in ["hue", "chroma", "lightness", "strokeHue", "amount", "array", "jitterHue", "strokeJitterLightness", "seed"] {
        assert!(number_field("color", key).is_some(), "{key}");
    }
    assert_eq!(number_field("color", "hue").unwrap().clamp(250.0), Some(100.0));
    assert!(per_copy("color", "hue") && !per_copy("color", "seed"));
    assert_eq!(expr_field("color", "lightnessExpr"), Some("lightness"));
}

#[test]
fn no_colour_rows_no_shifts() {
    let out = eval(json!([{"kind": "linearArray", "count": 4}]));
    assert!(!out.has_colour() && out.colours.is_empty() && out.colour(2).is_empty());
    let out = eval(json!([{"kind": "linearArray", "count": 4}, {"kind": "color", "enabled": false}]));
    assert!(!out.has_colour());
}

#[test]
fn the_ramp_runs_across_all_copies_from_the_original() {
    let out = eval(json!([{"kind": "linearArray", "count": 5}, {"kind": "color", "hue": 40, "chroma": 0, "lightness": -20}]));
    assert_eq!(out.colours.len(), 5);
    assert_eq!(hues(&out), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    assert!(out.colours[0][0].is_zero(), "copy 0 keeps its colour");
    assert_eq!(out.colours[4][0].fill, OklchShift::new(40.0, 0.0, -20.0));
    // Strokes follow the fill by default.
    assert_eq!(out.colours[3][0].stroke, Some(out.colours[3][0].fill));
    // Wherever the row sits: above the array it still counts the final copies.
    let above = eval(json!([{"kind": "color", "hue": 40, "chroma": 0, "lightness": -20}, {"kind": "linearArray", "count": 5}]));
    assert_eq!(above.colours, out.colours);
    // Amount, reverse.
    let half = eval(json!([{"kind": "linearArray", "count": 3}, {"kind": "color", "hue": 40, "amount": 0.5, "reverse": true}]));
    assert_eq!(hues(&half), vec![20.0, 10.0, 0.0]);
    // Ramp off: the full shift everywhere.
    let flat = eval(json!([{"kind": "linearArray", "count": 3}, {"kind": "color", "hue": 40, "progressive": false}]));
    assert_eq!(hues(&flat), vec![40.0; 3]);
}

#[test]
fn a_lone_object_takes_amount() {
    let out = eval(json!([{"kind": "color", "hue": 30, "amount": 0.5}]));
    assert_eq!(out.instances.len(), 1);
    assert_eq!(hues(&out), vec![15.0]);
    assert!(out.has_colour());
    // Out of range values are clamped, with a note.
    let out = eval(json!([{"id": "c", "kind": "color", "hue": 250, "amount": 3}]));
    assert_eq!(hues(&out), vec![100.0]);
    assert!(out.warnings.iter().any(|w| w.message.contains("`hue`")));
}

#[test]
fn stroke_modes() {
    let own = eval(json!([{"kind": "linearArray", "count": 3}, {"kind": "color", "hue": 20, "stroke": "own", "strokeLightness": 30}]));
    assert_eq!(own.colours[2][0].stroke, Some(OklchShift::new(0.0, 0.0, 30.0)));
    assert_eq!(own.colours[1][0].stroke, Some(OklchShift::new(0.0, 0.0, 15.0)));
    let none = eval(json!([{"kind": "linearArray", "count": 3}, {"kind": "color", "stroke": "none"}]));
    assert_eq!(none.colours[2][0].stroke, None);
}

#[test]
fn two_rows_chain_in_order() {
    let out = eval(json!([{"kind": "linearArray", "count": 2}, {"kind": "color", "hue": 10}, {"kind": "color", "hue": -50, "progressive": false}]));
    assert_eq!(out.colours[1].len(), 2);
    assert_eq!((out.colours[1][0].fill.hue, out.colours[1][1].fill.hue), (10.0, -50.0));
}

#[test]
fn order_array_ramps_along_one_array_row() {
    // A 1 × 3 grid, then a ring of 4: 12 copies.
    let mods = |array: u32| {
        json!([{"kind": "gridArray", "rows": 1, "cols": 3}, {"kind": "circularArray", "count": 4},
               {"kind": "color", "hue": 60, "order": "array", "array": array}])
    };
    let out = eval(mods(1));
    assert_eq!(out.colours.len(), 12);
    // Each ring repeats the grid's gradient.
    assert_eq!(hues(&out), [0.0, 30.0, 60.0].repeat(4));
    let out = eval(mods(2));
    let want: Vec<f64> = [0.0, 20.0, 40.0, 60.0].iter().flat_map(|h| [*h; 3]).collect();
    assert_eq!(hues(&out), want, "every grid in the ring gets one step");
    // Clamped to the arrays there are.
    assert_eq!(eval(mods(9)).colours, eval(mods(2)).colours);
    // No array rows: as `copies`.
    let lone = eval(json!([{"kind": "color", "order": "array", "array": 2, "hue": 50}]));
    assert_eq!(hues(&lone), vec![50.0]);
}

#[test]
fn position_orders_sort_by_the_copies_centres() {
    // Copies run right to left: left-right order ramps the other way.
    let out = eval(json!([{"kind": "linearArray", "count": 3, "offsetX": -50}, {"kind": "color", "hue": 40, "order": "leftRight"}]));
    assert_eq!(hues(&out), vec![40.0, 20.0, 0.0]);
    let tb = eval(json!([{"kind": "linearArray", "count": 3, "offsetY": -50}, {"kind": "color", "hue": 40, "order": "topBottom"}]));
    assert_eq!(hues(&tb), vec![40.0, 20.0, 0.0]);
    // Ties keep copy order.
    let tie = eval(json!([{"kind": "linearArray", "count": 3, "offsetX": 0}, {"kind": "color", "hue": 40, "order": "diagonal"}]));
    assert_eq!(hues(&tie), vec![0.0, 20.0, 40.0]);
}

#[test]
fn jitter_is_per_copy_and_channel_and_stable() {
    let mods = |count: u32, jl: f64| json!([{"kind": "linearArray", "count": count}, {"kind": "color", "hue": 0, "chroma": 0, "lightness": 0, "jitterHue": 20, "jitterLightness": jl}]);
    let a = eval(mods(5, 0.0));
    let b = eval(mods(9, 10.0));
    for k in 0..5 {
        // Adding copies after it or turning on another channel doesn't change a copy's hue jitter.
        assert_eq!(a.colours[k][0].fill.hue, b.colours[k][0].fill.hue, "{k}");
        assert!(a.colours[k][0].fill.hue.abs() <= 20.0);
        assert_eq!(a.colours[k][0].fill.chroma, 0.0);
    }
    let distinct: std::collections::BTreeSet<u64> = a.colours.iter().map(|c| c[0].fill.hue.to_bits()).collect();
    assert_eq!(distinct.len(), 5);
    // The seed changes it.
    let c = eval(json!([{"kind": "linearArray", "count": 5}, {"kind": "color", "hue": 0, "jitterHue": 20, "seed": 7}]));
    assert_ne!(c.colours[1][0].fill.hue, a.colours[1][0].fill.hue);
    // Stroke jitter with its own channels.
    let s = eval(json!([{"kind": "linearArray", "count": 3}, {"kind": "color", "hue": 0, "jitterHue": 20, "stroke": "own", "strokeJitterHue": 20}]));
    assert_ne!(s.colours[1][0].stroke.unwrap().hue, s.colours[1][0].fill.hue);
}

#[test]
fn expressions_replace_the_ramp() {
    let out = eval(json!([{"kind": "linearArray", "count": 4}, {"kind": "color", "hue": 10, "hueExpr": "i * 25", "lightnessExpr": "value"}]));
    assert_eq!(hues(&out), vec![0.0, 25.0, 50.0, 75.0]);
    assert!(out.colours.iter().all(|c| c[0].fill.lightness == 10.0), "value = the field, no ramp");
    // Results are clamped to the field's range.
    let out = eval(json!([{"kind": "linearArray", "count": 2}, {"kind": "color", "hueExpr": "i * 500"}]));
    assert_eq!(hues(&out), vec![0.0, 100.0]);
    // A bound per-copy expression.
    let st = stack(
        json!([{"kind": "linearArray", "count": 3}, {"id": "c", "kind": "color", "bind": {"hue": {"expr": "i * 7"}, "chroma": {"param": "c"}}}]),
    );
    let scope = Scope::from_values([("c".to_string(), ParamValue::Number(40.0))]);
    let out = evaluate_in(&st, B, &NoTargets, &scope);
    assert_eq!(hues(&out), vec![0.0, 7.0, 14.0]);
    assert_eq!(out.colours[2][0].fill.chroma, 40.0, "a bound single value ramps like the field");
    assert!(out.bound.iter().any(|b| b.field == "hue" && b.per_copy));
}

#[test]
fn a_colour_row_breaks_the_stencil_lattice_and_adds_no_geometry() {
    let out = eval(json!([{"kind": "gridArray", "rows": 2, "cols": 2}, {"kind": "color"}, {"kind": "stencil", "output": "tile"}]));
    assert!(out.repeat.is_none());
    assert!(!out.has_geometry());
    let plain = eval(json!([{"kind": "gridArray", "rows": 2, "cols": 2}]));
    let coloured = eval(json!([{"kind": "gridArray", "rows": 2, "cols": 2}, {"kind": "color"}]));
    assert_eq!(plain.instances, coloured.instances, "nothing moves");
}

/// The shifts are plain arithmetic: the same bits natively and in wasm32. This hash is the golden
/// value for a 200-copy stack with ramp, jitter and an order.
#[test]
fn two_hundred_copies_golden() {
    let out = eval(json!([
        {"kind": "gridArray", "rows": 10, "cols": 20, "gapX": 13, "gapY": 7},
        {"kind": "color", "hue": 70, "chroma": -30, "lightness": 12.5, "amount": 0.8, "jitterHue": 9, "jitterChroma": 4, "jitterLightness": 3,
         "stroke": "own", "strokeHue": -40, "strokeJitterLightness": 6, "seed": 1234},
        {"kind": "color", "hue": 5, "order": "diagonal", "reverse": true}
    ]));
    assert_eq!(out.colours.len(), 200);
    let mut h = 0u64;
    for c in out.colours.iter().flatten() {
        let s = c.stroke.unwrap_or_default();
        for v in [c.fill.hue, c.fill.chroma, c.fill.lightness, s.hue, s.chroma, s.lightness] {
            h = splitmix64(h ^ v.to_bits());
        }
    }
    assert_eq!(h, 14_546_229_682_016_597_469, "golden hash");
}
