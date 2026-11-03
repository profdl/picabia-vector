//! PICABIA: the physics transport bar and the clones on the canvas (PIC31.5), drawn headless.

use egui::Rect;
use serde_json::json;
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;

/// A jelly and a box on a 400 × 300 artboard, and a context with the canvas where the app puts it.
fn scene() -> (VectorcraftApp, egui::Context, u64, u64) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let jelly = app.run("shape.ellipse", json!({"x": 40, "y": 30, "width": 80, "height": 60})).unwrap()["id"].as_u64().unwrap();
    app.run("physics.set", json!({"ids": [jelly], "props": {"bodyType": "soft", "soft": {"preset": "jelly"}}})).unwrap();
    let boxy = app.run("shape.rectangle", json!({"x": 200, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("physics.set", json!({"ids": [boxy]})).unwrap();
    app.canvas_rect = Some(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 700.0)));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    (app, ctx, jelly, boxy)
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let time = ctx.input(|i| i.time) + 1.0 / 60.0;
    let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| super::sim_bar::show(app, ui.ctx()));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    // A new area is laid out unseen on its first frame.
    frame(app, ctx, vec![]);
    let shown = frame(app, ctx, vec![]);
    let at = shown.iter().rev().find(|(s, _)| s == label).unwrap_or_else(|| panic!("no {label:?} in {shown:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        frame(app, ctx, events);
    }
}

fn texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]).into_iter().map(|(s, _)| s).collect()
}

#[test]
fn no_bar_without_a_session_and_the_bar_with_one() {
    let (mut app, ctx, ..) = scene();
    assert!(texts(&mut app, &ctx).is_empty());
    app.run("physics.session.open", json!({})).unwrap();
    let shown = texts(&mut app, &ctx);
    for label in ["0.00 s", "To Rest", "Apply", "Cancel"] {
        assert!(shown.iter().any(|s| s == label), "no {label:?} in {shown:?}");
    }
    // It sits at the bottom of the canvas, inside it: where its texts are drawn.
    let canvas = app.canvas_rect.unwrap();
    frame(&mut app, &ctx, vec![]);
    let drawn = frame(&mut app, &ctx, vec![]);
    for (text, r) in drawn.iter().filter(|(t, _)| ["Apply", "Cancel", "To Rest"].contains(&t.as_str())) {
        assert!(canvas.contains_rect(*r) && r.top() > canvas.center().y, "{text} at {r:?}");
    }
    // On a narrow canvas it still fits.
    app.canvas_rect = Some(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(420.0, 500.0)));
    for _ in 0..4 {
        frame(&mut app, &ctx, vec![]);
    }
    let drawn = frame(&mut app, &ctx, vec![]);
    let cancel = drawn.iter().find(|(t, _)| t == "Cancel").unwrap().1;
    assert!(cancel.right() <= 420.0, "{cancel:?}");
}

#[test]
fn playing_steps_with_the_frames_and_the_canvas_shows_clones() {
    let (mut app, ctx, jelly, _) = scene();
    app.run("physics.play", json!({})).unwrap();
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    let info = app.run("physics.session.info", json!({})).unwrap();
    assert!(info["step"].as_u64().unwrap() >= 30, "{info}");
    // The document is unchanged; the canvas draws a clone of each moved body over its dimmed object.
    let st = app.session.active().unwrap();
    let shown = super::sim_bar::display_doc(&app).unwrap();
    let count = |d: &vectorcraft_doc::Document| d.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum::<usize>();
    assert_eq!(count(&shown), count(&st.doc) + 2, "a clone for each body");
    assert!(shown.node(NodeId(jelly)).unwrap().opacity < st.doc.node(NodeId(jelly)).unwrap().opacity, "the original is dimmed");
    assert!(st.history.undo.last().is_none_or(|e| e.label != "Apply Physics"));
}

#[test]
fn to_rest_then_apply_from_the_bar() {
    let (mut app, ctx, jelly, boxy) = scene();
    app.run("physics.session.open", json!({})).unwrap();
    click(&mut app, &ctx, "To Rest");
    assert!(app.run("physics.session.info", json!({})).unwrap()["settled"].as_bool().unwrap());
    click(&mut app, &ctx, "Apply");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.last().unwrap().label, "Apply Physics");
    for id in [jelly, boxy] {
        let b = st.doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert!((b.y1 - 300.0).abs() < 8.0, "{id}: {b:?}");
    }
    assert!(texts(&mut app, &ctx).is_empty(), "Apply closes the bar");
}

#[test]
fn cancel_closes_the_bar_and_leaves_the_art() {
    let (mut app, ctx, jelly, _) = scene();
    let before = app.session.active().unwrap().doc.clone();
    app.run("physics.toRest", json!({})).unwrap();
    click(&mut app, &ctx, "Cancel");
    assert!(app.session.active().unwrap().doc == before);
    assert!(super::sim_bar::display_doc(&app).is_none());
    assert!(texts(&mut app, &ctx).is_empty(), "the bar is gone");
    assert!(app.session.active().unwrap().doc.node(NodeId(jelly)).is_some());
}

#[test]
fn an_edit_during_a_session_never_shows_stale_clones() {
    let (mut app, ctx, jelly, _) = scene();
    app.run("physics.session.open", json!({})).unwrap();
    app.run("physics.seek", json!({"step": 60})).unwrap();
    assert!(super::sim_bar::display_doc(&app).is_some());
    // Delete the jelly: until the session follows (the next frame), the document itself is drawn.
    app.run("select.set", json!({"ids": [jelly]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    assert!(super::sim_bar::display_doc(&app).is_none(), "no stale copy");
    let rev = app.session.active().unwrap().revision;
    frame(&mut app, &ctx, vec![]);
    assert!(app.session.active().unwrap().revision > rev, "the restart redraws");
    let shown = super::sim_bar::display_doc(&app).unwrap();
    assert!(shown.node(NodeId(jelly)).is_none(), "the deleted jelly is gone from the canvas");
}
