//! `stencil`.
//!
//! Terminal and geometric: it makes no copies here (the evaluator stops at it and reports its
//! params in [`crate::Output::stencil`]); `vectorcraft-stencil` computes the sheet from what the rows
//! above drew. This module only puts the params in range and applies the material presets.

use serde_json::{Map, Value};

use super::Clamp;
use crate::model::{
    MAX_CORNER_ANGLE, MAX_HOSHI_DIAMETER_MM, MAX_PATTERN_SPACING_MM, MAX_TIE_WIDTH_MM, MAX_TIES_PER_ISLAND, MIN_HOSHI_DIAMETER_MM,
    MIN_PATTERN_SPACING_MM, MIN_TIE_WIDTH_MM, Stencil, StencilColourMode, StencilMaterial, hex_colour, parse_hex_colour,
};

/// `a` degrees in `0..360` (the same direction).
fn wrap_degrees(a: f64) -> f64 {
    let w = a.rem_euclid(360.0) + 0.0;
    if w >= 360.0 { 0.0 } else { w }
}

/// [`Stencil::clamped`]: every number in its range (non-finite: the default), `separate` as
/// `threshold`, colours normalised; a warning per change.
pub(crate) fn clamped(p: &Stencil) -> (Stencil, Vec<String>) {
    let d = Stencil::default();
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let threshold = c.within("threshold", p.threshold, 0.0, 1.0, d.threshold);
    let width_mm = c.within("widthMm", p.width_mm, 1.0, 10_000.0, d.width_mm);
    let margin_mm = c.within("marginMm", p.margin_mm, 0.0, 1000.0, d.margin_mm);
    let min_cut_area_mm2 = c.within("minCutAreaMm2", p.min_cut_area_mm2, 0.0, 10_000.0, d.min_cut_area_mm2);
    let min_cut_width_mm = c.within("minCutWidthMm", p.min_cut_width_mm, 0.0, 100.0, d.min_cut_width_mm);
    let min_bridge_mm = c.within("minBridgeMm", p.min_bridge_mm, 0.0, 100.0, d.min_bridge_mm);
    let tolerance_mm = c.within("toleranceMm", p.tolerance_mm, 0.0, 10.0, d.tolerance_mm);
    let pattern_width_mm = c.within("patternWidthMm", p.pattern_width_mm, MIN_TIE_WIDTH_MM, MAX_TIE_WIDTH_MM, d.pattern_width_mm);
    let mut pattern_spacing_mm =
        c.within("patternSpacingMm", p.pattern_spacing_mm, MIN_PATTERN_SPACING_MM, MAX_PATTERN_SPACING_MM, d.pattern_spacing_mm);
    let pattern_angle = if p.pattern_angle.is_finite() {
        wrap_degrees(p.pattern_angle)
    } else {
        c.warnings.push("`patternAngle` isn't a finite number; using 0".into());
        0.0
    };
    if pattern_spacing_mm < 2.0 * pattern_width_mm {
        c.warnings.push("`patternSpacingMm` is at least twice `patternWidthMm`".into());
        pattern_spacing_mm = 2.0 * pattern_width_mm;
    }
    let tie_width_mm = c.within("tieWidthMm", p.tie_width_mm, MIN_TIE_WIDTH_MM, MAX_TIE_WIDTH_MM, d.tie_width_mm);
    let min_corner_angle = c.within("minCornerAngle", p.min_corner_angle, 0.0, MAX_CORNER_ANGLE, d.min_corner_angle);
    let hoshi_diameter_mm = c.within("hoshiDiameterMm", p.hoshi_diameter_mm, MIN_HOSHI_DIAMETER_MM, MAX_HOSHI_DIAMETER_MM, d.hoshi_diameter_mm);
    let colour_mode = match p.colour_mode {
        StencilColourMode::Separate => {
            warnings.push("one stencil per colour (`separate`) isn't available yet; cutting by threshold".into());
            StencilColourMode::Threshold
        }
        m => m,
    };
    let mut colour = |name: &str, v: &str, default: &str| match parse_hex_colour(v) {
        Some(c) => hex_colour(c),
        None => {
            warnings.push(format!("`{name}` isn't a colour; using {default}"));
            default.to_string()
        }
    };
    let paper_colour = colour("paperColour", &p.paper_colour, &d.paper_colour);
    let paste_colour = colour("pasteColour", &p.paste_colour, &d.paste_colour);
    let cloth_colour = colour("clothColour", &p.cloth_colour, &d.cloth_colour);
    let mut cut_colours: Vec<String> = Vec::new();
    let mut bad = 0usize;
    for c in &p.cut_colours {
        match parse_hex_colour(c).map(hex_colour) {
            Some(c) if !cut_colours.contains(&c) => cut_colours.push(c),
            Some(_) => {}
            None => bad += 1,
        }
    }
    if bad > 0 {
        warnings.push(format!("{bad} of `cutColours` aren't colours; left out"));
    }
    if cut_colours.len() > crate::model::MAX_CUT_COLOURS {
        cut_colours.truncate(crate::model::MAX_CUT_COLOURS);
        warnings.push(format!("`cutColours` is limited to {} colours", crate::model::MAX_CUT_COLOURS));
    }
    if !(1..=MAX_TIES_PER_ISLAND).contains(&p.ties_per_island) {
        warnings.push(format!("`tiesPerIsland` is limited to 1..{MAX_TIES_PER_ISLAND}"));
    }
    let ties_per_island = p.ties_per_island.clamp(1, MAX_TIES_PER_ISLAND);
    let tie_angle = match p.tie_angle {
        Some(a) if a.is_finite() => Some(wrap_degrees(a)),
        Some(_) => {
            warnings.push("`tieAngle` isn't a finite number; no preferred direction".into());
            None
        }
        None => None,
    };
    let out = Stencil {
        colour_mode,
        cut_colours,
        threshold,
        invert: p.invert,
        width_mm,
        margin_mm,
        min_cut_area_mm2,
        min_cut_width_mm,
        min_bridge_mm,
        tolerance_mm,
        preview: p.preview,
        paper_colour,
        paste_colour,
        cloth_colour,
        tie_mode: p.tie_mode,
        tie_width_mm,
        ties_per_island,
        tie_spread: p.tie_spread,
        tie_angle,
        tie_seed: p.tie_seed,
        pattern_type: p.pattern_type,
        pattern_width_mm,
        pattern_spacing_mm,
        pattern_angle,
        pattern_where: p.pattern_where,
        material: p.material,
        limit_mode: p.limit_mode,
        min_corner_angle,
        hoshi: p.hoshi,
        hoshi_diameter_mm,
        repeat: p.repeat,
        output: p.output,
    };
    (out, warnings)
}

/// The four fields a material preset fills, by their JSON names.
const LIMIT_FIELDS: [&str; 4] = ["minCutWidthMm", "minBridgeMm", "tieWidthMm", "minCornerAngle"];

/// [`crate::Modifier::set_props`] on a stencil, `old` → `new` from `props`: setting `material` to a
/// preset fills the four limit fields that `props` doesn't set itself; setting one of them
/// (without `material`) to a value off the current preset makes the material `custom`.
pub(crate) fn apply_material(old: &Stencil, new: &mut Stencil, props: &Map<String, Value>) {
    let sets = |k: &str| props.contains_key(k);
    if sets("material") {
        if let Some(l) = new.material.limits() {
            let fields = [
                (&mut new.min_cut_width_mm, l.min_cut_width_mm),
                (&mut new.min_bridge_mm, l.min_bridge_mm),
                (&mut new.tie_width_mm, l.tie_width_mm),
                (&mut new.min_corner_angle, l.min_corner_angle),
            ];
            for ((field, preset), name) in fields.into_iter().zip(LIMIT_FIELDS) {
                if !sets(name) {
                    *field = preset;
                }
            }
        }
        return;
    }
    let Some(l) = old.material.limits() else { return };
    let values = [
        (new.min_cut_width_mm, l.min_cut_width_mm),
        (new.min_bridge_mm, l.min_bridge_mm),
        (new.tie_width_mm, l.tie_width_mm),
        (new.min_corner_angle, l.min_corner_angle),
    ];
    let off_preset = values.into_iter().zip(LIMIT_FIELDS).any(|((v, preset), name)| sets(name) && (v - preset).abs() > 1e-9);
    if off_preset {
        new.material = StencilMaterial::Custom;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_geom::{Affine, Rect};

    use crate::*;

    const B: Rect = Rect::new(0.0, 0.0, 20.0, 10.0);

    fn stack(mods: Value) -> Stack {
        parse(&json!({"v": 1, "stack": mods}).to_string()).unwrap()
    }

    fn stencil_of(row: Value) -> Stencil {
        match Modifier::from_value(&row).unwrap().kind {
            ModifierKind::Stencil(p) => p,
            k => panic!("not a stencil: {k:?}"),
        }
    }

    #[test]
    fn defaults_match_the_contract() {
        let m = Modifier::new("m1", "stencil").unwrap();
        assert_eq!(m.label(), "Stencil");
        assert_eq!(kind_label("stencil"), Some("Stencil"));
        let v = m.to_value();
        assert_eq!(
            v,
            json!({"id": "m1", "enabled": true, "kind": "stencil", "colourMode": "threshold", "cutColours": [], "threshold": 0.5,
                "invert": false, "widthMm": 300.0, "marginMm": 15.0, "minCutAreaMm2": 0.5, "minCutWidthMm": 0.3, "minBridgeMm": 0.5,
                "toleranceMm": 0.05, "preview": "paper", "paperColour": "#c8a165", "pasteColour": "#1f3a8a", "clothColour": "#f4efe6",
                "tieMode": "ties", "tieWidthMm": 1.0, "tiesPerIsland": 2, "tieSpread": true, "tieAngle": null, "tieSeed": 1,
                "patternType": "grid", "patternWidthMm": 0.6, "patternSpacingMm": 8.0, "patternAngle": 0.0, "patternWhere": "everywhere",
                "material": "paper", "limitMode": "fix", "minCornerAngle": 30.0, "hoshi": true, "hoshiDiameterMm": 1.5, "repeat": "auto",
                "output": "sheet"})
        );
        assert_eq!(Stencil::default().clamped(), (Stencil::default(), Vec::new()), "defaults are in range");
    }

    #[test]
    fn round_trip_keeps_values_and_unknown_fields() {
        let row = json!({"id": "s", "enabled": true, "kind": "stencil", "colourMode": "pick", "cutColours": ["#FFFFFF", "#0a0", "#ffffff", "nope", 7],
            "threshold": 0.25, "invert": true, "widthMm": 450, "preview": "cloth", "paperColour": "#ABCDEF", "tieWidthMm": 0.8, "tieAngle": "-90", "tieMode": "gauze", "tiesText": "flag", "future": {"a": [1]}});
        let m = Modifier::from_value(&row).unwrap();
        let ModifierKind::Stencil(p) = &m.kind else { panic!("stencil") };
        assert_eq!(p.colour_mode, StencilColourMode::Pick);
        assert_eq!(p.cut_colours, ["#ffffff", "#00aa00"], "normalised, invalid dropped, repeats once");
        assert_eq!((p.threshold, p.invert, p.width_mm, p.preview), (0.25, true, 450.0, StencilPreview::Cloth));
        assert_eq!(p.paper_colour, "#abcdef");
        assert_eq!((p.tie_width_mm, p.tie_angle, p.tie_mode), (0.8, Some(-90.0), StencilTieMode::Gauze));
        assert_eq!(m.extra.get("tiesText"), Some(&json!("flag")), "later fields kept");
        let back = m.to_value();
        assert_eq!(back["future"], json!({"a": [1]}));
        assert_eq!(Modifier::from_value(&back).unwrap(), m, "a second round trip changes nothing");
        let s = stack(json!([row]));
        assert_eq!(parse(&to_json(&s)).unwrap(), s);
    }

    #[test]
    fn lenient_parsing() {
        let p = stencil_of(json!({"kind": "stencil", "colourMode": "rainbow", "preview": 3, "paperColour": "brown", "pasteColour": null,
            "clothColour": 12, "cutColours": "#123456", "threshold": "0.75"}));
        assert_eq!(p.colour_mode, StencilColourMode::Threshold, "unknown choice: default");
        assert_eq!(p.preview, StencilPreview::Paper);
        assert_eq!((p.paper_colour.as_str(), p.paste_colour.as_str(), p.cloth_colour.as_str()), (STENCIL_PAPER, STENCIL_PASTE, STENCIL_CLOTH));
        assert_eq!(p.cut_colours, ["#123456"], "one colour reads as a list of one");
        assert_eq!(p.threshold, 0.75, "numeric strings read as numbers");
        let many: Vec<String> = (0..400).map(|i| format!("#{:06x}", i * 1000)).collect();
        let p = stencil_of(json!({"kind": "stencil", "cutColours": many}));
        assert_eq!(p.cut_colours.len(), MAX_CUT_COLOURS);
        let p = stencil_of(json!({"kind": "stencil", "colourMode": "separate", "cutColours": {"a": 1}}));
        assert_eq!(p.colour_mode, StencilColourMode::Separate, "separate reads and writes");
        assert!(p.cut_colours.is_empty());
        // A wrongly typed flag doesn't read: the row is kept as it was, unevaluated.
        let m = Modifier::from_value(&json!({"id": "s", "kind": "stencil", "invert": "yes"})).unwrap();
        assert!(matches!(&m.kind, ModifierKind::Other { kind, error: Some(_) } if kind == "stencil"));
        assert_eq!(m.to_value()["invert"], json!("yes"));
    }

    #[test]
    fn clamping_and_non_finite_values() {
        let p = Stencil {
            colour_mode: StencilColourMode::Separate,
            threshold: 3.0,
            width_mm: 0.0,
            margin_mm: f64::NAN,
            min_cut_area_mm2: -1.0,
            min_cut_width_mm: f64::INFINITY,
            min_bridge_mm: 1e9,
            tolerance_mm: f64::NEG_INFINITY,
            paper_colour: "zz".into(),
            cut_colours: vec!["#ABC".into(), "x".into()],
            ..Stencil::default()
        };
        let (c, w) = p.clamped();
        let d = Stencil::default();
        assert_eq!(c.colour_mode, StencilColourMode::Threshold, "separate evaluates as threshold");
        assert_eq!((c.threshold, c.width_mm, c.margin_mm, c.min_cut_area_mm2), (1.0, 1.0, d.margin_mm, 0.0));
        assert_eq!((c.min_cut_width_mm, c.min_bridge_mm, c.tolerance_mm), (d.min_cut_width_mm, 100.0, d.tolerance_mm));
        assert_eq!(c.paper_colour, STENCIL_PAPER);
        assert_eq!(c.cut_colours, ["#aabbcc"]);
        assert_eq!(w.len(), 10, "{w:?}");
        let (c, _) = Stencil { width_mm: 1e12, threshold: -1.0, ..Stencil::default() }.clamped();
        assert_eq!((c.width_mm, c.threshold), (10_000.0, 0.0));
    }

    #[test]
    fn a_stencil_alone_draws_the_layer_unchanged() {
        let out = evaluate(&stack(json!([{"id": "s", "kind": "stencil"}])), B);
        assert_eq!(out.instances.len(), 1);
        assert_eq!(out.instances[0].transform, Affine::IDENTITY);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.stencil, Some(StencilRow { modifier: "s".into(), params: Stencil::default() }));
        // A disabled stencil is no stencil: nothing active, draw normally.
        let out = evaluate(&stack(json!([{"id": "s", "enabled": false, "kind": "stencil"}])), B);
        assert!(out.instances.is_empty() && out.stencil.is_none());
        // Through a node: saves, loads and draws once, as the original.
        let rect = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 10.0));
        let mut n = vectorcraft_doc::Node::path(vectorcraft_doc::NodeId(3), rect, vectorcraft_doc::Appearance::default_art());
        crate::set_node_source(&mut n, Some(to_json(&stack(json!([{"id": "s", "kind": "stencil", "widthMm": 120}])))));
        let out = node_instances(&n).unwrap();
        assert_eq!(out.transforms().collect::<Vec<_>>(), [Affine::IDENTITY]);
        assert_eq!(out.stencil.as_ref().map(|s| s.params.width_mm), Some(120.0));
    }

    #[test]
    fn the_stencil_is_terminal() {
        let s = stack(json!([
            {"id": "a", "kind": "linearArray", "count": 3},
            {"id": "s", "kind": "stencil", "threshold": 2},
            {"id": "b", "kind": "gridArray", "rows": 4, "cols": 4},
            {"id": "c", "enabled": false, "kind": "mirror"},
            {"id": "d", "kind": "spiralArray"},
        ]));
        let out = evaluate(&s, B);
        assert_eq!(out.instances.len(), 3, "the copies above stand, the rows below add none");
        let st = out.stencil.as_ref().unwrap();
        assert_eq!((st.modifier.as_str(), st.params.threshold), ("s", 1.0));
        let below: Vec<&Warning> = out.warnings.iter().filter(|w| w.message.contains("below it are ignored")).collect();
        assert_eq!(below.len(), 1, "one warning: {:?}", out.warnings);
        assert!(below[0].message.contains("2 rows"), "the disabled row isn't counted: {}", below[0].message);
        assert_eq!(below[0].modifier.as_deref(), Some("s"));
        assert!(!out.warnings.iter().any(|w| w.modifier.as_deref() == Some("d")), "the unknown row below isn't evaluated");
        assert_eq!(s.stencil().map(|(id, p)| (id, p.threshold)), Some(("s", 2.0)), "Stack::stencil has the params as written");
        // The first enabled stencil ends the stack; one below it is just another ignored row.
        let out = evaluate(&stack(json!([{"id": "s1", "kind": "stencil"}, {"id": "s2", "kind": "stencil", "invert": true}])), B);
        assert_eq!(out.stencil.as_ref().map(|s| s.modifier.as_str()), Some("s1"));
        assert!(out.warnings.iter().any(|w| w.message.contains("the 1 row below")), "{:?}", out.warnings);
        // A per-copy row above still applies to the copies.
        let out = evaluate(&stack(json!([{"kind": "linearArray", "count": 2}, {"kind": "transformEach", "offsetX": 5}, {"kind": "stencil"}])), B);
        assert_eq!(out.instances.len(), 2);
        assert!((out.instances[0].transform.translation().x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_stencil_below_a_capped_row_still_ends_the_stack() {
        let s = stack(json!([
            {"id": "a", "kind": "linearArray", "count": 1000},
            {"id": "b", "kind": "linearArray", "count": 1000},
            {"id": "s", "kind": "stencil"},
            {"id": "c", "kind": "mirror"},
        ]));
        let out = evaluate(&s, B);
        assert_eq!(out.instances.len(), 1000);
        assert_eq!(out.stencil.as_ref().map(|s| s.modifier.as_str()), Some("s"));
        assert!(out.warnings.iter().any(|w| w.message.contains("the 1 row below")), "{:?}", out.warnings);
    }

    #[test]
    fn set_props_edits_a_stencil() {
        let mut m = Modifier::new("m1", "stencil").unwrap();
        m.set_props(json!({"colourMode": "pick", "cutColours": ["#FF0000"], "widthMm": "200", "invert": true}).as_object().unwrap()).unwrap();
        let ModifierKind::Stencil(p) = &m.kind else { panic!("stencil") };
        assert_eq!(
            (p.colour_mode, p.cut_colours.as_slice(), p.width_mm, p.invert),
            (StencilColourMode::Pick, ["#ff0000".to_string()].as_slice(), 200.0, true)
        );
        let before = m.clone();
        for bad in [
            json!({"widthMm": "NaN"}),
            json!({"invert": 1}),
            json!({"tieText": 1}),
            json!({"widthMm": null}),
            json!({"tieAngle": "north"}),
            json!({"tieSpread": 0}),
        ] {
            assert!(m.set_props(bad.as_object().unwrap()).is_err(), "{bad}");
            assert_eq!(m, before);
        }
    }

    #[test]
    fn tie_fields() {
        let p = stencil_of(json!({"kind": "stencil", "tieMode": "silk", "tiesPerIsland": 3.6, "tieAngle": null, "tieSeed": -4, "tieSpread": false}));
        assert_eq!((p.tie_mode, p.ties_per_island, p.tie_angle, p.tie_seed, p.tie_spread), (StencilTieMode::Ties, 4, None, 0, false));
        assert_eq!(stencil_of(json!({"kind": "stencil", "tieAngle": ""})).tie_angle, None, "a blank text is no angle");
        assert_eq!(stencil_of(json!({"kind": "stencil", "tieAngle": 45})).tie_angle, Some(45.0));
        let m = Modifier::from_value(&json!({"kind": "stencil", "tieAngle": [1]})).unwrap();
        assert!(matches!(&m.kind, ModifierKind::Other { error: Some(_), .. }), "a mistyped angle doesn't read");
        // Clamping.
        let (c, w) = Stencil { tie_width_mm: 0.0, ties_per_island: 0, tie_angle: Some(-30.0), ..Stencil::default() }.clamped();
        assert_eq!((c.tie_width_mm, c.ties_per_island, c.tie_angle), (MIN_TIE_WIDTH_MM, 1, Some(330.0)));
        assert_eq!(w.len(), 2, "wrapping the angle isn't a warning: {w:?}");
        let (c, w) = Stencil { tie_width_mm: f64::NAN, ties_per_island: 1000, tie_angle: Some(f64::INFINITY), ..Stencil::default() }.clamped();
        assert_eq!((c.tie_width_mm, c.ties_per_island, c.tie_angle), (1.0, MAX_TIES_PER_ISLAND, None));
        assert_eq!(w.len(), 3, "{w:?}");
        for (a, want) in [(720.0, 0.0), (-0.0, 0.0), (-1e-20, 0.0), (359.5, 359.5), (1e300, 1e300f64.rem_euclid(360.0))] {
            let (c, _) = Stencil { tie_angle: Some(a), ..Stencil::default() }.clamped();
            let got = c.tie_angle.unwrap();
            assert!((0.0..360.0).contains(&got) && got.to_bits() == want.to_bits(), "{a} → {got}");
        }
        let (c, _) = Stencil { tie_width_mm: 1e9, ..Stencil::default() }.clamped();
        assert_eq!(c.tie_width_mm, MAX_TIE_WIDTH_MM);
        // Editing: an angle can be set and cleared.
        let mut m = Modifier::new("m1", "stencil").unwrap();
        m.set_props(json!({"tieAngle": 90, "tiesPerIsland": 5, "tieMode": "gauze"}).as_object().unwrap()).unwrap();
        let ModifierKind::Stencil(p) = &m.kind else { panic!("stencil") };
        assert_eq!((p.tie_angle, p.ties_per_island, p.tie_mode), (Some(90.0), 5, StencilTieMode::Gauze));
        m.set_props(json!({"tieAngle": null}).as_object().unwrap()).unwrap();
        let ModifierKind::Stencil(p) = &m.kind else { panic!("stencil") };
        assert_eq!(p.tie_angle, None);
    }

    #[test]
    fn pattern_fields() {
        let p =
            stencil_of(json!({"kind": "stencil", "tieMode": "pattern", "patternType": "vertical", "patternWidthMm": "0.8", "patternSpacingMm": 5,
            "patternAngle": 30, "patternWhere": "islands"}));
        assert_eq!(
            (p.tie_mode, p.pattern_type, p.pattern_where),
            (StencilTieMode::Pattern, StencilPatternType::Vertical, StencilPatternWhere::Islands)
        );
        assert_eq!((p.pattern_width_mm, p.pattern_spacing_mm, p.pattern_angle), (0.8, 5.0, 30.0));
        assert_eq!(p.clamped(), (p.clone(), Vec::new()), "in range");
        let p = stencil_of(json!({"kind": "stencil", "patternType": "hex", "patternWhere": 4}));
        assert_eq!((p.pattern_type, p.pattern_where), (StencilPatternType::Grid, StencilPatternWhere::Everywhere), "unknown choices: defaults");
        let (c, w) = Stencil { pattern_width_mm: 3.0, pattern_spacing_mm: 4.0, pattern_angle: -90.0, ..Stencil::default() }.clamped();
        assert_eq!((c.pattern_width_mm, c.pattern_spacing_mm, c.pattern_angle), (3.0, 6.0, 270.0), "spacing ≥ 2 × width");
        assert_eq!(w.len(), 1, "{w:?}");
        let (c, w) = Stencil { pattern_width_mm: 1e6, pattern_spacing_mm: 0.0, pattern_angle: f64::NAN, ..Stencil::default() }.clamped();
        assert_eq!((c.pattern_width_mm, c.pattern_spacing_mm, c.pattern_angle), (MAX_TIE_WIDTH_MM, 2.0 * MAX_TIE_WIDTH_MM, 0.0));
        assert_eq!(w.len(), 4, "{w:?}");
        let (c, _) = Stencil { pattern_spacing_mm: 1e9, ..Stencil::default() }.clamped();
        assert_eq!(c.pattern_spacing_mm, MAX_PATTERN_SPACING_MM);
        let _ = MIN_PATTERN_SPACING_MM;
    }

    #[test]
    fn material_limit_hoshi_repeat_and_output_fields() {
        let p = stencil_of(json!({"kind": "stencil", "material": "film", "limitMode": "flag", "minCornerAngle": "45", "hoshi": false,
            "hoshiDiameterMm": 3, "repeat": "off", "output": "tile"}));
        assert_eq!(
            (p.material, p.limit_mode, p.min_corner_angle, p.hoshi, p.hoshi_diameter_mm, p.repeat, p.output),
            (StencilMaterial::Film, StencilLimitMode::Flag, 45.0, false, 3.0, StencilRepeat::Off, StencilOutput::Tile)
        );
        assert_eq!(p.clamped(), (p.clone(), Vec::new()), "in range");
        let m = Modifier::from_value(&json!({"kind": "stencil", "material": "film", "output": "tile"})).unwrap();
        let back = m.to_value();
        assert_eq!((back["material"].clone(), back["output"].clone()), (json!("film"), json!("tile")));
        assert_eq!(Modifier::from_value(&back).unwrap(), m, "round trip");
        // Unknown choices read as the defaults.
        let p = stencil_of(json!({"kind": "stencil", "material": "silk", "limitMode": 3, "repeat": "always", "output": "roll"}));
        assert_eq!(
            (p.material, p.limit_mode, p.repeat, p.output),
            (StencilMaterial::Paper, StencilLimitMode::Fix, StencilRepeat::Auto, StencilOutput::Sheet)
        );
        // Clamping.
        let (c, w) = Stencil { min_corner_angle: 200.0, hoshi_diameter_mm: 0.0, ..Stencil::default() }.clamped();
        assert_eq!((c.min_corner_angle, c.hoshi_diameter_mm), (MAX_CORNER_ANGLE, MIN_HOSHI_DIAMETER_MM));
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = Stencil { min_corner_angle: f64::NAN, hoshi_diameter_mm: 1e9, ..Stencil::default() }.clamped();
        assert_eq!((c.min_corner_angle, c.hoshi_diameter_mm), (30.0, MAX_HOSHI_DIAMETER_MM));
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, _) = Stencil { min_corner_angle: -5.0, ..Stencil::default() }.clamped();
        assert_eq!(c.min_corner_angle, 0.0);
        assert!(Modifier::from_value(&json!({"kind": "stencil", "hoshi": 1})).is_ok_and(|m| matches!(m.kind, ModifierKind::Other { .. })));
    }

    fn limits(m: &Modifier) -> (StencilMaterial, f64, f64, f64, f64) {
        let ModifierKind::Stencil(p) = &m.kind else { panic!("stencil") };
        (p.material, p.min_cut_width_mm, p.min_bridge_mm, p.tie_width_mm, p.min_corner_angle)
    }

    fn set(m: &mut Modifier, props: Value) {
        m.set_props(props.as_object().unwrap()).unwrap();
    }

    #[test]
    fn set_props_applies_material_presets() {
        use StencilMaterial::*;
        let mut m = Modifier::new("m1", "stencil").unwrap();
        set(&mut m, json!({"material": "film"}));
        assert_eq!(limits(&m), (Film, 0.2, 0.4, 0.6, 20.0), "a preset fills the four fields");
        set(&mut m, json!({"material": "card", "minBridgeMm": 2}));
        assert_eq!(limits(&m), (Card, 0.5, 2.0, 1.5, 35.0), "fields set alongside it win");
        set(&mut m, json!({"material": "film"}));
        set(&mut m, json!({"tieWidthMm": 0.6, "widthMm": 400}));
        assert_eq!(limits(&m).0, Film, "the preset's own value and other fields keep it");
        set(&mut m, json!({"minCornerAngle": "21"}));
        assert_eq!(limits(&m), (Custom, 0.2, 0.4, 0.6, 21.0), "a value off the preset makes it custom");
        set(&mut m, json!({"minCutWidthMm": 0.2}));
        assert_eq!(limits(&m).0, Custom, "custom stays custom, even at a preset's value");
        set(&mut m, json!({"material": "custom"}));
        assert_eq!(limits(&m), (Custom, 0.2, 0.4, 0.6, 21.0), "custom fills nothing");
        set(&mut m, json!({"material": "paper"}));
        assert_eq!(limits(&m), (Paper, 0.3, 0.5, 1.0, 30.0), "paper is the defaults again");
        set(&mut m, json!({"material": "silk"}));
        assert_eq!(limits(&m), (Paper, 0.3, 0.5, 1.0, 30.0), "an unknown material reads as paper");
        set(&mut m, json!({"minBridgeMm": 0.6, "material": "card"}));
        assert_eq!(limits(&m), (Card, 0.5, 0.6, 1.5, 35.0), "with `material` in the props, no switch to custom");
        // A refused edit changes nothing.
        let before = m.clone();
        assert!(m.set_props(json!({"material": "film", "minBridgeMm": "wide"}).as_object().unwrap()).is_err());
        assert_eq!(m, before);
        // Bound values are applied without it: the stored material stands.
        assert_eq!(Modifier::from_value(&json!({"kind": "stencil", "material": "film", "tieWidthMm": 3})).map(|m| limits(&m).0).unwrap(), Film);
    }

    fn repeat_of(mods: Value) -> Option<Repeat> {
        evaluate(&stack(mods), B).repeat
    }

    fn rep(ox: f64, oy: f64, a: (f64, f64), b: (f64, f64)) -> Option<Repeat> {
        Some(Repeat {
            origin: vectorcraft_geom::Point::new(ox, oy),
            a: vectorcraft_geom::Vec2::new(a.0, a.1),
            b: vectorcraft_geom::Vec2::new(b.0, b.1),
        })
    }

    #[test]
    fn a_grid_above_the_stencil_gives_its_repeat() {
        let grid = json!({"kind": "gridArray", "cols": 3, "rows": 2, "gapX": 5, "gapY": 2});
        assert_eq!(repeat_of(json!([grid, {"kind": "stencil"}])), rep(0.0, 0.0, (25.0, 0.0), (0.0, 12.0)), "steps of bounds + gap");
        let b = Rect::new(10.0, 20.0, 30.0, 30.0);
        assert_eq!(evaluate(&stack(json!([grid, {"kind": "stencil"}])), b).repeat, rep(10.0, 20.0, (25.0, 0.0), (0.0, 12.0)), "at the top-left");
        let column = json!([{"kind": "gridArray", "cols": 1, "rows": 3, "gapX": 0, "gapY": 0}, {"kind": "stencil"}]);
        assert_eq!(repeat_of(column), rep(0.0, 0.0, (0.0, 0.0), (0.0, 10.0)), "one column: no repeat along a");
        let row = json!([{"kind": "gridArray", "cols": 4, "rows": 1, "gapX": 0, "gapY": 0}, {"kind": "stencil"}]);
        assert_eq!(repeat_of(row), rep(0.0, 0.0, (20.0, 0.0), (0.0, 0.0)), "one row: no repeat along b");
        assert_eq!(repeat_of(json!([{"kind": "gridArray", "cols": 1, "rows": 1}, {"kind": "stencil"}])), None, "1 × 1: no repeat");
    }

    #[test]
    fn a_wallpaper_above_the_stencil_gives_its_lattice() {
        let wp = json!({"kind": "wallpaper", "group": "p4m", "cols": 2, "rows": 2, "cellWidth": 100, "cellHeight": 70, "originX": 3, "originY": 4});
        assert_eq!(repeat_of(json!([wp, {"kind": "stencil"}])), rep(3.0, 4.0, (100.0, 0.0), (0.0, 100.0)), "square: w by w at the origin");
        let hex = json!({"kind": "wallpaper", "group": "p6m", "cols": 1, "rows": 1, "cellWidth": 100});
        let r = repeat_of(json!([hex, {"kind": "stencil"}])).unwrap();
        assert_eq!((r.a, r.b.x), (vectorcraft_geom::Vec2::new(100.0, 0.0), 50.0), "the lattice even for one cell");
        assert!((r.b.y - 50.0 * 3f64.sqrt()).abs() < 1e-9);
        // The last lattice row counts; a motif row before it is fine.
        let s =
            json!([{"kind": "mirror"}, {"kind": "gridArray", "cols": 2, "rows": 2}, wp, {"kind": "linearArray", "count": 1}, {"kind": "stencil"}]);
        assert_eq!(repeat_of(s), rep(3.0, 4.0, (100.0, 0.0), (0.0, 100.0)), "mirror and grid inside the motif, one copy after");
    }

    #[test]
    fn no_repeat_without_an_intact_lattice() {
        let grid = json!({"kind": "gridArray", "cols": 3, "rows": 2});
        assert_eq!(repeat_of(json!([grid, {"kind": "stencil", "repeat": "off"}])), None, "repeat off");
        assert_eq!(repeat_of(json!([grid])), None, "no stencil");
        assert_eq!(repeat_of(json!([grid, {"kind": "stencil", "enabled": false}])), None, "a disabled stencil");
        assert_eq!(repeat_of(json!([{"kind": "linearArray", "count": 3}, {"kind": "stencil"}])), None, "no lattice row");
        for after in [
            json!({"kind": "transformEach", "rotate": 10}),
            json!({"kind": "randomize", "offsetX": 3}),
            json!({"kind": "linearArray", "count": 2, "offsetX": 7}),
            json!({"kind": "circularArray", "count": 3}),
            json!({"kind": "mirror"}),
        ] {
            assert_eq!(repeat_of(json!([grid, after, {"kind": "stencil"}])), None, "{after} after the lattice row");
        }
        // Per-copy rows vary every final copy wherever they sit.
        assert_eq!(repeat_of(json!([{"kind": "transformEach", "rotate": 10}, grid, {"kind": "stencil"}])), None);
        // A disabled or unknown row after it doesn't count.
        let s = json!([grid, {"kind": "transformEach", "enabled": false, "rotate": 10}, {"kind": "spiral"}, {"kind": "stencil"}]);
        assert!(repeat_of(s).is_some());
    }

    #[test]
    fn tile_output_needs_a_repeat() {
        let tile = json!({"id": "s", "kind": "stencil", "output": "tile"});
        let out = evaluate(&stack(json!([{"kind": "gridArray", "cols": 2}, tile])), B);
        assert_eq!(out.stencil.as_ref().map(|s| s.params.output), Some(StencilOutput::Tile));
        assert!(out.repeat.is_some() && out.warnings.is_empty(), "{:?}", out.warnings);
        for s in [json!([tile]), json!([{"kind": "gridArray", "cols": 2}, {"id": "s", "kind": "stencil", "output": "tile", "repeat": "off"}])] {
            let out = evaluate(&stack(s), B);
            assert_eq!(out.stencil.as_ref().map(|s| s.params.output), Some(StencilOutput::Sheet), "evaluates as sheet");
            let w: Vec<&Warning> = out.warnings.iter().filter(|w| w.message.contains("tile output needs a repeat")).collect();
            assert_eq!((w.len(), w[0].modifier.as_deref()), (1, Some("s")), "{:?}", out.warnings);
        }
    }

    #[test]
    fn material_presets() {
        let paper = StencilMaterial::Paper.limits().unwrap();
        let d = Stencil::default();
        assert_eq!(
            (paper.min_cut_width_mm, paper.min_bridge_mm, paper.tie_width_mm, paper.min_corner_angle),
            (d.min_cut_width_mm, d.min_bridge_mm, d.tie_width_mm, d.min_corner_angle),
            "paper is exactly the defaults"
        );
        assert_eq!(paper.kerf_mm, 0.10);
        let film = StencilMaterial::Film.limits().unwrap();
        assert_eq!((film.min_cut_width_mm, film.min_bridge_mm, film.tie_width_mm, film.min_corner_angle, film.kerf_mm), (0.2, 0.4, 0.6, 20.0, 0.08));
        let card = StencilMaterial::Card.limits().unwrap();
        assert_eq!((card.min_cut_width_mm, card.min_bridge_mm, card.tie_width_mm, card.min_corner_angle, card.kerf_mm), (0.5, 1.0, 1.5, 35.0, 0.15));
        assert_eq!(StencilMaterial::Custom.limits(), None);
        for m in StencilMaterial::ALL {
            assert_eq!(serde_json::to_value(m).unwrap(), json!(m.id()));
            if let Some(l) = m.limits() {
                let p = Stencil {
                    min_cut_width_mm: l.min_cut_width_mm,
                    min_bridge_mm: l.min_bridge_mm,
                    tie_width_mm: l.tie_width_mm,
                    min_corner_angle: l.min_corner_angle,
                    ..d.clone()
                };
                assert_eq!(p.clamped().1, Vec::<String>::new(), "{m:?}'s preset is in range");
            }
        }
    }

    #[test]
    fn hex_colours() {
        assert_eq!(parse_hex_colour("#C8A165"), Some([200, 161, 101]));
        assert_eq!(parse_hex_colour(" fff "), Some([255, 255, 255]));
        for bad in ["", "#", "#12345", "#1234567", "#gggggg", "#ééé", "#12é4"] {
            assert_eq!(parse_hex_colour(bad), None, "{bad}");
        }
        assert_eq!(hex_colour([31, 58, 138]), STENCIL_PASTE);
    }
}
