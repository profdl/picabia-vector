//! PICABIA: the binding chip on the modifier rows' number fields and the Variables panel's
//! Parameters section, drawn headless.

use egui::{Event, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_modifiers::{Binding, BoundField, Parameters, Scope};

use super::bind_chip::{Chip, Look, shown_value};
use crate::VectorcraftApp;
use crate::panels::set_pstate;

struct Frames {
    ctx: egui::Context,
    time: f64,
}

impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        Self { ctx, time: 0.0 }
    }

    fn run(&mut self, app: &mut VectorcraftApp, events: Vec<Event>, panel: fn(&mut VectorcraftApp, &mut egui::Ui)) -> egui::FullOutput {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(320.0, 900.0));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| panel(app, ui));
        out.textures_delta.clear();
        out
    }

    /// The accessibility nodes of a fresh frame: (role, label, centre, focusable).
    fn nodes(&mut self, app: &mut VectorcraftApp, panel: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<(egui::accesskit::Role, String, Pos2, bool)> {
        self.run(app, vec![], panel);
        let out = self.run(app, vec![], panel);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        nodes
            .iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                let c = pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
                Some((n.role(), n.label().unwrap_or_default().to_string(), c, n.supports_action(egui::accesskit::Action::Focus)))
            })
            .collect()
    }

    fn at(&mut self, app: &mut VectorcraftApp, panel: fn(&mut VectorcraftApp, &mut egui::Ui), want: &dyn Fn(&str) -> bool) -> Pos2 {
        let nodes = self.nodes(app, panel);
        nodes
            .iter()
            .find(|(_, l, _, _)| want(l))
            .map(|n| n.2)
            .unwrap_or_else(|| panic!("no node: {:?}", nodes.iter().map(|n| &n.1).collect::<Vec<_>>()))
    }

    fn click(&mut self, app: &mut VectorcraftApp, at: Pos2, panel: fn(&mut VectorcraftApp, &mut egui::Ui)) {
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.run(app, vec![Event::PointerMoved(at)], panel);
        self.run(app, vec![button(true)], panel);
        self.run(app, vec![button(false)], panel);
        self.run(app, vec![], panel);
    }

    fn texts(&mut self, app: &mut VectorcraftApp, panel: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<String> {
        fn collect(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        self.run(app, vec![], panel);
        let out = self.run(app, vec![], panel);
        let mut v = vec![];
        out.shapes.iter().for_each(|c| collect(&c.shape, &mut v));
        v
    }
}

fn appearance(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::panels::appearance::show(app, ui);
}

fn variables(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::panels::variables::show(app, ui);
}

/// A selected rectangle with a linear array whose count is bound to `copies` (4); its row open.
fn bound(f: &Frames) -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("param.add", json!({"name": "copies", "value": 4, "min": 1, "max": 12})).unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "linearArray", "props": {"count": 2, "offsetX": 40}})).unwrap();
    app.run("modifier.bind", json!({"ids": [id], "index": 0, "field": "count", "binding": {"param": "copies"}})).unwrap();
    set_pstate(&f.ctx, &format!("pc-open-{id}-m1"), true);
    (app, id)
}

fn list(app: &mut VectorcraftApp, id: u64) -> Value {
    app.run("modifier.list", json!({"id": id})).unwrap()
}

#[test]
fn the_chip_shows_each_state() {
    let params = Parameters::default();
    let scope = Scope::default();
    let failed =
        BoundField { modifier: "m1".into(), field: "count".into(), value: None, per_copy: false, error: Some("no parameter named `x`".into()) };
    let per = BoundField { modifier: "m1".into(), field: "rotate".into(), value: None, per_copy: true, error: None };
    let ok = BoundField { modifier: "m1".into(), field: "count".into(), value: Some(4.0), per_copy: false, error: None };
    let (p, e, o) = (Binding::Param("copies".into()), Binding::Expr("i * 15".into()), Binding::Other(json!({"ramp": 1})));
    let chip = |binding: Option<&'static Binding>, bound: Option<&'static BoundField>| (binding, bound);
    let p: &'static Binding = Box::leak(Box::new(p));
    let e: &'static Binding = Box::leak(Box::new(e));
    let o: &'static Binding = Box::leak(Box::new(o));
    let (failed, per, ok): (&'static BoundField, &'static BoundField, &'static BoundField) =
        (Box::leak(Box::new(failed)), Box::leak(Box::new(per)), Box::leak(Box::new(ok)));
    for ((binding, bound), look, tip) in [
        (chip(None, None), Look::Free, "Bind Copies to a parameter or an expression"),
        (chip(Some(p), Some(ok)), Look::Param, "Copies = parameter copies"),
        (chip(Some(e), Some(per)), Look::Expr, "Copies = i * 15\nA value per copy"),
        (chip(Some(p), Some(failed)), Look::Failed, "Copies = parameter copies\nno parameter named `x`"),
        (chip(Some(o), None), Look::Failed, "Copies: a binding this version can't use"),
    ] {
        let c = Chip { id: egui::Id::unique("x"), label: "Copies", binding, bound, per_copy_field: false, params: &params, scope: &scope };
        assert_eq!((c.look(), c.tip().as_str()), (look, tip));
    }
    let fmt = |v: f64| format!("{v:.0}");
    assert_eq!(shown_value(None, Some(ok), 2.0, &fmt), None, "unbound: the editor shows");
    assert_eq!(shown_value(Some(p), Some(ok), 2.0, &fmt).as_deref(), Some("4"));
    assert_eq!(shown_value(Some(p), Some(failed), 2.0, &fmt).as_deref(), Some("2"), "failed: its own value");
    assert_eq!(shown_value(Some(e), Some(per), 2.0, &fmt).as_deref(), Some("per copy"));
}

#[test]
fn a_bound_field_shows_its_value_and_a_keyboard_reachable_chip() {
    let mut f = Frames::new();
    let (mut app, _) = bound(&f);
    let texts = f.texts(&mut app, appearance);
    assert!(texts.iter().any(|s| s == "4"), "the bound count shows the parameter's value: {texts:?}");
    let nodes = f.nodes(&mut app, appearance);
    let chip = nodes.iter().find(|n| n.1 == "Copies = parameter copies").expect("the bound field's chip");
    assert_eq!(chip.0, egui::accesskit::Role::Button);
    assert!(chip.3, "the chip takes keyboard focus");
    for field in ["Offset X", "Offset Y", "Rotate each", "Scale each"] {
        let want = format!("Bind {field} to a parameter or an expression");
        assert!(nodes.iter().any(|n| n.1 == want), "{field}: every number field has a chip");
    }
}

/// Every number field the panel shows can be bound, with the range the panel clamps to, and every
/// field a binding may drive is shown.
#[test]
fn the_panels_number_fields_are_the_bindable_ones_with_the_same_ranges() {
    use super::appearance::{F, KINDS, fields};
    for (kind, _) in KINDS.iter().filter(|(k, _)| *k != "stencil") {
        let mut shown = vec![];
        for (key, _, how) in fields(kind) {
            let range = match *how {
                F::Len(lo, hi) | F::Deg(lo, hi) | F::Pct(lo, hi) | F::Int(lo, hi) | F::Num(lo, hi) => (lo, hi),
                F::Bool | F::Choice(_) | F::Target => continue,
            };
            let spec = vectorcraft_modifiers::number_field(kind, key).unwrap_or_else(|| panic!("{kind}.{key} can't be bound"));
            assert_eq!((spec.min, spec.max), range, "{kind}.{key}");
            assert_eq!(spec.whole, matches!(how, F::Int(..)), "{kind}.{key} whole");
            shown.push(*key);
        }
        let bindable: Vec<&str> = vectorcraft_modifiers::number_fields(kind).iter().map(|f| f.key).collect();
        assert_eq!(shown, bindable, "{kind}");
    }
}

#[test]
fn the_stencil_settings_have_chips() {
    let mut f = Frames::new();
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("param.add", json!({"name": "w", "value": 120, "max": 500})).unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "stencil"})).unwrap();
    app.run("modifier.bind", json!({"ids": [id], "index": 0, "field": "widthMm", "binding": {"param": "w"}})).unwrap();
    set_pstate(&f.ctx, &format!("pc-open-{id}-m1"), true);
    let nodes = f.nodes(&mut app, appearance);
    assert!(nodes.iter().any(|n| n.1 == "Width = parameter w"), "{:?}", nodes.iter().map(|n| &n.1).collect::<Vec<_>>());
    assert!(nodes.iter().any(|n| n.1 == "Bind Margin to a parameter or an expression"));
    assert!(f.texts(&mut app, appearance).iter().any(|s| s == "120 mm"), "the bound width shows the parameter's value");
}

#[test]
fn the_popover_binds_an_expression_and_unbinds() {
    let mut f = Frames::new();
    let (mut app, id) = bound(&f);
    let chip = f.at(&mut app, appearance, &|l| l == "Bind Offset X to a parameter or an expression");
    f.click(&mut app, chip, appearance);
    let texts = f.texts(&mut app, appearance);
    assert!(texts.iter().any(|s| s == "Bind Offset X") && texts.iter().any(|s| s == "Expression"), "{texts:?}");
    // Type an expression into the popover's field and commit it.
    let field = f.at(&mut app, appearance, &|l| l == "Expression");
    f.click(&mut app, field, appearance);
    let enter = Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    f.run(&mut app, vec![Event::Text("copies * 10".into())], appearance);
    f.run(&mut app, vec![enter], appearance);
    f.run(&mut app, vec![], appearance);
    let l = list(&mut app, id);
    assert_eq!(l["modifiers"][0]["bind"]["offsetX"], json!({"expr": "copies * 10"}), "{l}");
    assert!(f.texts(&mut app, appearance).iter().any(|s| s.contains("40")), "the bound offset shows 40 pt");
    // Unbind from the popover.
    let chip = f.at(&mut app, appearance, &|l| l == "Offset X = copies * 10");
    f.click(&mut app, chip, appearance);
    let unbind = f.at(&mut app, appearance, &|l| l == "Unbind");
    f.click(&mut app, unbind, appearance);
    assert!(list(&mut app, id)["modifiers"][0]["bind"].get("offsetX").is_none());
}

#[test]
fn a_failed_binding_warns_on_its_chip() {
    let mut f = Frames::new();
    let (mut app, _) = bound(&f);
    app.run("param.remove", json!({"name": "copies"})).unwrap();
    let nodes = f.nodes(&mut app, appearance);
    assert!(
        nodes.iter().any(|n| n.1.starts_with("Copies = parameter copies\n") && n.1.contains("no parameter named `copies`")),
        "{:?}",
        nodes.iter().map(|n| &n.1).collect::<Vec<_>>()
    );
    assert!(f.texts(&mut app, appearance).iter().any(|s| s == "2"), "the field's own value");
}

#[test]
fn the_variables_panel_lists_parameters_and_adds_them() {
    let mut f = Frames::new();
    let (mut app, id) = bound(&f);
    let texts = f.texts(&mut app, variables);
    assert!(texts.iter().any(|s| s == "Parameters"), "{texts:?}");
    assert!(texts.iter().any(|s| s == "copies"), "{texts:?}");
    let nodes = f.nodes(&mut app, variables);
    assert!(nodes.iter().any(|n| n.0 == egui::accesskit::Role::Slider), "a number parameter has a slider");
    let add = f.at(&mut app, variables, &|l| l == "+ Number");
    f.click(&mut app, add, variables);
    let names: Vec<Value> = app.run("param.list", json!({})).unwrap()["parameters"].as_array().unwrap().iter().map(|p| p["name"].clone()).collect();
    assert_eq!(names, vec![json!("copies"), json!("param1")]);
    // Typing a value in the parameter's field sets it, and the bound object follows.
    let field = f.at(&mut app, variables, &|l| l == "copies");
    f.click(&mut app, field, variables);
    let select_all = Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    let enter = Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    f.run(&mut app, vec![select_all], variables);
    f.run(&mut app, vec![Event::Text("9".into())], variables);
    f.run(&mut app, vec![enter], variables);
    f.run(&mut app, vec![], variables);
    assert_eq!(list(&mut app, id)["instances"], json!(9));
}
