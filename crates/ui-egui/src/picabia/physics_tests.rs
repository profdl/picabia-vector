//! PICABIA: the physics row in the Appearance panel and Object ▸ Physics, drawn headless.

use egui::Rect;
use serde_json::json;

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::appearance::show;

/// A selected rectangle with physics settings, and the panel's context after a frame.
fn with_physics() -> (VectorcraftApp, egui::Context, u64) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
    app.run("physics.set", json!({"ids": [id]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    (app, ctx, id)
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let time = ctx.input(|i| i.time) + 0.1;
    let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let shown = frame(app, ctx, vec![]);
    let at = shown.iter().rev().find(|(s, _)| s == label).unwrap_or_else(|| panic!("no {label:?} in {shown:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        frame(app, ctx, events);
    }
}

fn texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    frame(app, ctx, vec![]).into_iter().map(|(s, _)| s).collect()
}

#[test]
fn the_row_shows_for_objects_with_settings_and_simulate_drops_them() {
    let (mut app, ctx, id) = with_physics();
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s == "Physics: Dynamic"), "{shown:?}");
    assert!(shown.iter().any(|s| s == "Simulate"));
    // Simulate to Rest: the session shows the result; nothing moves until Apply (PIC31.5).
    let undo = app.session.active().unwrap().history.undo.len();
    click(&mut app, &ctx, "Simulate");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    assert!(app.ui.status.contains("settled"), "{}", app.ui.status);
    app.run("physics.apply", json!({})).unwrap();
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert!((b.y1 - 300.0).abs() < 2.0, "{b:?}");
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Apply Physics");
}

#[test]
fn the_fields_open_and_edit_the_settings() {
    let (mut app, ctx, id) = with_physics();
    assert!(!texts(&mut app, &ctx).iter().any(|s| s == "Bounce"));
    app.picabia.physics_open = true;
    let shown = texts(&mut app, &ctx);
    for label in ["Body", "Collider", "Gravity", "Bounce", "Friction", "Extra mass", "Drag", "Spin drag", "Sensor (passes through)", "Clear Physics"]
    {
        assert!(shown.iter().any(|s| s == label), "no {label:?} in {shown:?}");
    }
    click(&mut app, &ctx, "Sensor (passes through)");
    assert_eq!(app.run("physics.get", json!({"id": id})).unwrap()["settings"]["sensor"], true);
    click(&mut app, &ctx, "Clear Physics");
    assert_eq!(app.run("physics.get", json!({"id": id})).unwrap()["settings"], serde_json::Value::Null);
    assert!(!texts(&mut app, &ctx).iter().any(|s| s.starts_with("Physics:")));
}

#[test]
fn no_row_without_settings() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    assert!(!texts(&mut app, &ctx).iter().any(|s| s.starts_with("Physics:")));
}

#[test]
fn object_physics_menu_runs_its_commands() {
    let tree = crate::menus::menu_tree();
    let object = tree.iter().find(|(title, _)| *title == "Object").map(|(_, items)| items).unwrap();
    let Some(Item::Sub(_, physics)) = object.iter().find(|i| matches!(i, Item::Sub("Physics", _))) else { panic!("no Object ▸ Physics") };
    let ids: Vec<&str> = physics.iter().filter_map(|i| if let Item::Cmd(_, id, _) = i { Some(*id) } else { None }).collect();
    assert_eq!(
        ids,
        [
            "physics.toRest",
            "physics.play",
            "physics.simulate",
            "physics.set",
            "physics.set",
            "physics.set",
            "physics.set",
            "physics.set",
            crate::picabia_ui::PHYSICS_SETTINGS,
            "physics.pin",
            "physics.pin",
            "physics.unpin",
            "physics.clear"
        ]
    );
    // Every item runs from the app.
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
    for item in physics {
        if let Item::Cmd(label, cmd, p) = item {
            let p = if p.is_null() { json!({}) } else { p.clone() };
            // Pinning needs anchors chosen on a soft object (tested in the engine).
            if !matches!(*cmd, "physics.clear" | "physics.pin") {
                app.run(cmd, p).unwrap_or_else(|e| panic!("{label}: {e}"));
            }
        }
    }
    let settings = app.run("physics.get", json!({"id": id})).unwrap()["settings"].clone();
    assert_eq!((settings["bodyType"].as_str(), settings["soft"]["preset"].as_str()), (Some("soft"), Some("rubber")), "{settings}");
}

#[test]
fn a_soft_body_shows_its_preset_and_fields() {
    let (mut app, ctx, id) = with_physics();
    app.run("physics.set", json!({"ids": [id], "props": {"bodyType": "soft", "soft": {"preset": "cloth"}}})).unwrap();
    app.picabia.physics_open = true;
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s == "Physics: Soft (Cloth)"), "{shown:?}");
    for label in ["Preset", "Stiffness", "Bend", "Wobble damping", "Detail", "Self-collide"] {
        assert!(shown.iter().any(|s| s == label), "no {label:?} in {shown:?}");
    }
    assert!(!shown.iter().any(|s| s == "Shape memory" || s == "Collider"), "cloth has no shape memory, soft bodies no collider: {shown:?}");
    click(&mut app, &ctx, "Self-collide");
    let settings = app.run("physics.get", json!({"id": id})).unwrap()["settings"].clone();
    assert_eq!(settings["soft"]["selfCollide"], true, "{settings}");
    // Jelly shows shape memory instead of bend.
    app.run("physics.set", json!({"ids": [id], "props": {"soft": {"preset": "jelly"}}})).unwrap();
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s == "Shape memory") && !shown.iter().any(|s| s == "Bend"), "{shown:?}");
}
