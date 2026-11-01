//! Per-copy expressions: the parser, the evaluator, storage in the model, and the
//! transformEach/randomize evaluators reading them.

use serde_json::{Map, Value, json};
use vectorcraft_geom::{Point, Rect};

use crate::expr::{Env, Expr, MAX_EXPR_DEPTH, MAX_EXPR_LEN, check};
use crate::{Modifier, ModifierKind, Stack, evaluate, parse, splitmix64, to_json};

fn ev(text: &str, env: Env) -> f64 {
    Expr::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")).eval(&env)
}

fn at(i: usize, n: usize) -> Env {
    Env::new(i, n, 10.0)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn arithmetic_precedence_and_associativity() {
    let e = Env::default();
    for (text, want) in [
        ("1 + 2 * 3", 7.0),
        ("(1 + 2) * 3", 9.0),
        ("10 - 4 - 3", 3.0),
        ("12 / 3 / 2", 2.0),
        ("2 ^ 3 ^ 2", 512.0),
        ("-2 ^ 2", -4.0),
        ("-3 * 2", -6.0),
        ("2 * -3", -6.0),
        ("--4", 4.0),
        ("+4", 4.0),
        ("7 % 3", 1.0),
        (".5 + 1e1 + 2E-1", 10.7),
        ("  1\t+\n1 ", 2.0),
    ] {
        assert!(close(ev(text, e), want), "{text} = {} (want {want})", ev(text, e));
    }
}

#[test]
fn variables_and_t() {
    assert_eq!(ev("i", at(3, 5)), 3.0);
    assert_eq!(ev("n", at(3, 5)), 5.0);
    assert_eq!(ev("t", at(2, 5)), 0.5);
    assert_eq!(ev("t", at(0, 1)), 0.0, "t is 0 for a single copy");
    assert_eq!(ev("value * 2", at(0, 1)), 20.0);
    assert!(close(ev("PI", at(0, 1)), std::f64::consts::PI), "names ignore case");
}

#[test]
fn functions() {
    let e = Env::default();
    for (text, want) in [
        ("sin(90)", 1.0),
        ("cos(180)", -1.0),
        ("tan(45)", 1.0),
        ("asin(1)", 90.0),
        ("atan2(1, 1)", 45.0),
        ("abs(-3)", 3.0),
        ("floor(2.7) + ceil(2.1) + round(2.5)", 8.0),
        ("sqrt(16)", 4.0),
        ("sign(-5) + sign(0)", -1.0),
        ("min(4, 2, 8)", 2.0),
        ("max(4)", 4.0),
        ("mod(-1, 4)", 3.0),
        ("clamp(15, 0, 10)", 10.0),
        ("lerp(10, 20, 0.25)", 12.5),
        ("deg(rad(30))", 30.0),
        ("ln(exp(2))", 2.0),
    ] {
        assert!(close(ev(text, e), want), "{text} = {} (want {want})", ev(text, e));
    }
    let r = ev("rand(3)", e);
    assert!((0.0..1.0).contains(&r));
    assert_eq!(r, ev("rand(3)", e), "deterministic");
    assert_eq!(ev("rand(0)", e), ev("rand(-0)", e));
    assert_ne!(r, ev("rand(4)", e));
}

#[test]
fn non_finite_results_fall_back() {
    let e = Env::default();
    for text in ["1 / 0", "sqrt(-1)", "0 / 0", "mod(1, 0)", "clamp(1, 5, 0)", "ln(0)", "10 ^ 400"] {
        let x = Expr::parse(text).unwrap();
        assert!(!x.eval(&e).is_finite(), "{text}");
        assert_eq!(x.eval_or(&e, 7.0), 7.0, "{text}");
    }
}

#[test]
fn errors_are_reported_with_positions() {
    for (text, part, pos) in [
        ("", "empty", 0),
        ("   ", "empty", 0),
        ("1 +", "ends too soon", 3),
        ("(1 + 2", "missing `)`", 6),
        ("1 + 2)", "after the expression", 5),
        ("foo + 1", "unknown name `foo`", 0),
        ("bar(1)", "unknown function `bar`", 0),
        ("sin", "needs parentheses", 0),
        ("sin(1, 2)", "takes 1 argument", 0),
        ("clamp(1)", "takes 3", 0),
        ("min()", "takes 1 to 8", 0),
        ("1 $ 2", "unexpected `$`", 2),
        ("1..2", "isn't a number", 0),
        ("2 * , 3", "unexpected `,`", 4),
        ("é", "unexpected `é`", 0),
    ] {
        let e = Expr::parse(text).err().unwrap_or_else(|| panic!("{text} parsed"));
        assert!(e.message.contains(part), "{text}: {e}");
        assert_eq!(e.at, pos, "{text}: {e}");
    }
    assert!(check(&"1+".repeat(200)).unwrap_err().message.contains("under 256"));
    let deep = format!("{}1{}", "(".repeat(MAX_EXPR_DEPTH + 1), ")".repeat(MAX_EXPR_DEPTH + 1));
    assert!(check(&deep).unwrap_err().message.contains("nested too deeply"));
    assert!(check(&"-".repeat(100)).is_err());
    assert!(check(&format!("{}1", "-".repeat(MAX_EXPR_DEPTH - 1))).is_ok());
    assert!(check("min(1,1,1,1,1,1,1,1,1)").is_err());
}

#[test]
fn varies_says_whether_copies_differ() {
    assert!(Expr::parse("i * 15").unwrap().varies());
    assert!(Expr::parse("rand(value)").unwrap().varies());
    assert!(!Expr::parse("value * 2 + sin(30)").unwrap().varies());
}

/// Random texts from the language's pieces and from arbitrary characters never panic, and every
/// one parses and evaluates quickly.
#[test]
fn fuzzed_texts_never_panic_and_stay_fast() {
    const PIECES: &[&str] = &[
        "i", "n", "t", "value", "pi", "0", "1", "2.5", ".5", "1e3", "1e400", "-", "+", "*", "/", "%", "^", "(", ")", ",", " ", "sin(", "min(",
        "max(", "clamp(", "mod(", "rand(", "atan2(", "sqrt(", "e", "E", "é", "\u{0}", "$", "..", "nan", "inf", "1e-400",
    ];
    let start = std::time::Instant::now();
    let mut state = 7u64;
    let mut next = || {
        state = splitmix64(state);
        state
    };
    let envs = [Env::default(), at(0, 1), at(5, 9), Env { i: f64::MAX, n: f64::NAN, t: f64::INFINITY, value: -0.0 }];
    let mut parsed = 0;
    for case in 0..20_000 {
        let len = (next() % 64) as usize;
        let text: String = if case % 4 == 3 {
            // Arbitrary characters.
            (0..len).filter_map(|_| char::from_u32((next() % 0x2FF) as u32)).collect()
        } else {
            (0..len).map(|_| PIECES[(next() % PIECES.len() as u64) as usize]).collect()
        };
        if let Ok(e) = Expr::parse(&text) {
            parsed += 1;
            for env in &envs {
                let v = e.eval_or(env, 1.0);
                assert!(v.is_finite(), "{text}");
            }
        }
    }
    assert!(parsed > 100, "the fuzzer should reach valid expressions too ({parsed})");
    assert!(start.elapsed().as_secs() < 20, "fuzzing took {:?}", start.elapsed());
}

/// The worst-case text the caps allow parses and evaluates in bounded time.
#[test]
fn the_largest_texts_are_bounded() {
    let chain = "i+".repeat(MAX_EXPR_LEN / 2 - 1) + "i";
    assert!(chain.len() <= MAX_EXPR_LEN);
    let nested = format!("{}i{}", "sin(".repeat(MAX_EXPR_DEPTH - 1), ")".repeat(MAX_EXPR_DEPTH - 1));
    let start = std::time::Instant::now();
    for text in [chain, nested] {
        let e = Expr::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
        for k in 0..10_000 {
            let _ = e.eval(&at(k, 10_000));
        }
    }
    assert!(start.elapsed().as_secs() < 5);
}

// The model: `<field>Expr` params.

fn modifier(v: Value) -> Modifier {
    Modifier::from_value(&v).unwrap()
}

fn props(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn expressions_are_stored_as_expr_params() {
    let mut m = Modifier::new("m1", "transformEach").unwrap();
    m.set_props(&props(json!({"rotateExpr": " i * 15 ", "scaleXExpr": 2}))).unwrap();
    assert_eq!(crate::expr::field_expr(&m.kind, "rotate").as_deref(), Some("i * 15"));
    assert_eq!(crate::expr::field_expr(&m.kind, "scaleX").as_deref(), Some("2"));
    let v = m.to_value();
    assert_eq!(v["rotateExpr"], "i * 15");
    assert_eq!(v["scaleXExpr"], "2", "kept as text");
    assert!(v.get("offsetXExpr").is_none(), "unset expressions aren't written");
    // A numeric-looking text stays text through a file round trip.
    let back = modifier(v.clone());
    assert_eq!(back, m);
    // Cleared by an empty text or null.
    m.set_props(&props(json!({"rotateExpr": "", "scaleXExpr": null}))).unwrap();
    assert!(m.to_value().get("rotateExpr").is_none() && m.to_value().get("scaleXExpr").is_none());
    assert_eq!(crate::expr::field_expr(&m.kind, "rotate"), None);
}

#[test]
fn bad_expressions_are_refused_by_set() {
    let mut m = Modifier::new("m1", "randomize").unwrap();
    let before = m.clone();
    for p in [
        json!({"rotationExpr": "i +"}),
        json!({"rotationExpr": [1]}),
        json!({"rotationExpr": "x".repeat(300)}),
        json!({"seedExpr": "i"}),
        json!({"rotateExpr": "i"}),
    ] {
        assert!(m.set_props(&props(p.clone())).is_err(), "{p}");
        assert_eq!(m, before, "{p}");
    }
    let mut lin = Modifier::new("m2", "linearArray").unwrap();
    assert!(lin.set_props(&props(json!({"countExpr": "i"}))).is_err(), "arrays take no expressions");
    assert_eq!(crate::expr_fields("linearArray"), &[] as &[&str]);
    assert_eq!(crate::expr_field("randomize", "scaleExpr"), Some("scale"));
}

#[test]
fn damaged_stored_expressions_keep_the_modifier_and_warn() {
    let src = r#"{"v":1,"stack":[{"id":"a","kind":"linearArray","count":3},{"id":"m1","kind":"transformEach","rotate":5,"rotateExpr":"i +"}]}"#;
    let stack = parse(src).unwrap();
    assert!(matches!(stack.modifiers[1].kind, ModifierKind::TransformEach(_)));
    let out = evaluate(&stack, Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(out.warnings.iter().any(|w| w.message.contains("rotateExpr")), "{:?}", out.warnings);
    assert_eq!(out.instances.len(), 3);
    // A non-text expression makes the row unreadable (kept raw), like any bad param.
    let src = r#"{"v":1,"stack":[{"id":"m1","kind":"transformEach","rotateExpr":{"x":1}}]}"#;
    let stack = parse(src).unwrap();
    assert!(matches!(stack.modifiers[0].kind, ModifierKind::Other { .. }));
    assert!(to_json(&stack).contains("rotateExpr"));
}

// The evaluators.

fn arrayed(each: Value) -> Stack {
    let mut s = Stack::default();
    s.insert(9, modifier(json!({"kind": "linearArray", "count": 5, "offsetX": 100}))).unwrap();
    s.insert(9, modifier(each)).unwrap();
    s
}

/// The rotation in degrees of an instance's linear part.
fn degrees(a: vectorcraft_geom::Affine) -> f64 {
    let c = a.as_coeffs();
    c[1].atan2(c[0]).to_degrees()
}

#[test]
fn rotate_by_i_times_15_turns_each_copy_15_degrees_more() {
    let s = arrayed(json!({"kind": "transformEach", "rotateExpr": "i * 15"}));
    let out = evaluate(&s, Rect::new(0.0, 0.0, 10.0, 10.0));
    let turns: Vec<f64> = out.instances.iter().map(|i| (degrees(i.transform) * 1e6).round() / 1e6).collect();
    assert_eq!(turns, [0.0, 15.0, 30.0, 45.0, 60.0]);
    // Each copy still turns around its own centre: positions follow the array.
    let centres: Vec<Point> = out.instances.iter().map(|i| i.transform * Point::new(5.0, 5.0)).collect();
    for (k, c) in centres.iter().enumerate() {
        assert!((c.x - (5.0 + 100.0 * k as f64)).abs() < 1e-9 && (c.y - 5.0).abs() < 1e-9, "{k}: {c:?}");
    }
}

#[test]
fn expressions_read_t_and_value_and_skip_progressive() {
    let s = arrayed(
        json!({"kind": "transformEach", "rotate": 90, "progressive": true, "rotateExpr": "value * t", "offsetYExpr": "i * 10", "scaleXExpr": "1 + t"}),
    );
    let out = evaluate(&s, Rect::new(0.0, 0.0, 10.0, 10.0));
    let turns: Vec<f64> = out.instances.iter().map(|i| (degrees(i.transform) * 1e6).round() / 1e6).collect();
    assert_eq!(turns, [0.0, 22.5, 45.0, 67.5, 90.0]);
    // offsetY ignores progressive: copy 4 moves 40 down.
    let last = out.instances[4].transform;
    let c = last * Point::new(5.0, 5.0);
    assert!((c.y - 45.0).abs() < 1e-9, "{c:?}");
    // scaleX = 2 on the last copy (the x axis's length).
    let k = last.as_coeffs();
    assert!(((k[0] * k[0] + k[1] * k[1]).sqrt() - 2.0).abs() < 1e-9);
}

#[test]
fn without_expressions_the_evaluators_are_unchanged() {
    let s = arrayed(json!({"kind": "transformEach", "rotate": 40, "offsetX": 7, "scaleX": 2, "progressive": true, "jitter": 0.3}));
    let mut with = s.clone();
    // An expression that gives each copy exactly what the ramp would.
    with.modifiers[1].set_props(&props(json!({"rotateExpr": "value * t"}))).unwrap();
    let (a, b) = (evaluate(&s, Rect::new(0.0, 0.0, 10.0, 10.0)), evaluate(&with, Rect::new(0.0, 0.0, 10.0, 10.0)));
    for (x, y) in a.instances.iter().zip(&b.instances) {
        let (p, q) = (x.transform.as_coeffs(), y.transform.as_coeffs());
        assert!(p.iter().zip(q).all(|(u, v)| (u - v).abs() < 1e-9), "{p:?} vs {q:?}");
    }
}

#[test]
fn randomize_amounts_follow_expressions() {
    // No randomness on the first copy (amount 0), full on the last.
    let s =
        arrayed(json!({"kind": "randomize", "position": 50, "rotation": 30, "seed": 4, "positionExpr": "value * t", "rotationExpr": "value * t"}));
    let out = evaluate(&s, Rect::new(0.0, 0.0, 10.0, 10.0));
    let first = out.instances[0].transform * Point::new(5.0, 5.0);
    assert!((first.x - 5.0).abs() < 1e-9 && (first.y - 5.0).abs() < 1e-9 && degrees(out.instances[0].transform).abs() < 1e-9);
    let plain = evaluate(&arrayed(json!({"kind": "randomize", "position": 50, "rotation": 30, "seed": 4})), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(out.instances[4].transform, plain.instances[4].transform, "t = 1: the field's own amount");
    // Out-of-range results are clamped, never negative or huge.
    let s = arrayed(json!({"kind": "randomize", "scale": 0.5, "scaleExpr": "-1e300 * i"}));
    assert!(evaluate(&s, Rect::new(0.0, 0.0, 10.0, 10.0)).instances.iter().all(|i| i.transform.is_finite()));
}
