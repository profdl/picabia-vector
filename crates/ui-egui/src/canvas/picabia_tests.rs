//! PICABIA: the modifier handles on the canvas, drawn and dragged headless, and the Appearance
//! panel choosing whose handles show.

use egui::{Pos2, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use super::super::{Xf, show};
use super::KNOB;
use crate::VectorcraftApp;

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut v));
    v
}

/// A 40 pt square at 40, 40 with a 4-copy linear array 60 apart (m1) and a mirror (m2), selected,
/// with the Selection tool, shown at `zoom`.
fn arrayed(zoom: f64) -> (VectorcraftApp, egui::Context, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "linearArray", "props": {"count": 4, "offsetX": 60}})).unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "mirror", "props": {"axis": "horizontal"}})).unwrap();
    app.session.select_tool("selection", Default::default()).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = zoom;
    frame(&mut app, &ctx, vec![]);
    (app, ctx, id)
}

fn xf(app: &VectorcraftApp) -> Xf {
    Xf::new(app.canvas_rect.unwrap(), app.view().unwrap())
}

/// A knob (a circle of the handle's size, either size) drawn at `at`.
fn knob_at(drawn: &[Shape], at: Pos2) -> bool {
    drawn.iter().any(|s| matches!(s, Shape::Circle(c) if c.center.distance(at) < 0.01 && c.radius >= KNOB - 0.01 && c.radius <= KNOB + 2.0))
}

fn offset_x(app: &mut VectorcraftApp, id: u64) -> f64 {
    app.run("modifier.list", json!({"id": id})).unwrap()["modifiers"][0]["offsetX"].as_f64().unwrap()
}

#[test]
fn the_canvas_draws_the_handles_of_the_open_row_else_the_last() {
    let (mut app, ctx, id) = arrayed(1.5);
    let x = xf(&app);
    // Pivot (60, 60): the linear end at 60 + 3 × 60; the horizontal mirror's knob to its left.
    let end = x.to_screen(Point::new(240.0, 60.0));
    let mirror = x.to_screen(Point::new(60.0 - 100.0, 60.0));
    let drawn = frame(&mut app, &ctx, vec![]);
    assert!(knob_at(&drawn, mirror) && !knob_at(&drawn, end), "the last row's handle");
    app.session.picabia_set_handle_rows(vectorcraft_doc::NodeId(id), vec!["m1".into()]);
    let drawn = frame(&mut app, &ctx, vec![]);
    assert!(knob_at(&drawn, end) && !knob_at(&drawn, mirror), "the open row's handle");
    // Another tool: none.
    app.session.select_tool("directSelection", Default::default()).unwrap();
    assert!(!knob_at(&frame(&mut app, &ctx, vec![]), end));
}

#[test]
fn dragging_a_handle_on_the_canvas_is_one_undo_step() {
    let (mut app, ctx, id) = arrayed(2.0);
    app.session.picabia_set_handle_rows(vectorcraft_doc::NodeId(id), vec!["m1".into()]);
    let x = xf(&app);
    let from = x.to_screen(Point::new(240.0, 60.0));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    // 60 screen px left at 2 px a point: the last copy 30 pt nearer, each step 10 pt shorter.
    for k in 1..=4 {
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + vec2(-15.0 * k as f32, 0.0))]);
    }
    frame(&mut app, &ctx, vec![button(from + vec2(-60.0, 0.0), false)]);
    assert!((offset_x(&mut app, id) - 50.0).abs() < 1e-3, "{}", offset_x(&mut app, id));
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 1, "still selected");
    assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, 40.0, "the object didn't move");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(offset_x(&mut app, id), 60.0, "one undo step");
}

#[test]
fn a_bound_handle_is_drawn_greyed() {
    let (mut app, ctx, id) = arrayed(1.0);
    app.session.picabia_set_handle_rows(vectorcraft_doc::NodeId(id), vec!["m1".into()]);
    app.run("param.add", json!({"name": "step", "value": 60})).unwrap();
    app.run("modifier.bind", json!({"ids": [id], "index": 0, "field": "offsetX", "binding": {"param": "step"}})).unwrap();
    let at = xf(&app).to_screen(Point::new(240.0, 60.0));
    let t = crate::theme::Tokens::get(&ctx);
    let drawn = frame(&mut app, &ctx, vec![]);
    let ring = drawn.iter().find_map(|s| match s {
        Shape::Circle(c) if c.center.distance(at) < 0.01 && c.stroke.width > 0.0 => Some(c.stroke.color),
        _ => None,
    });
    assert_eq!(ring, Some(t.text_disabled));
}

#[test]
fn the_appearance_panel_reports_its_open_rows() {
    let (mut app, ctx, id) = arrayed(1.0);
    let names = |app: &mut VectorcraftApp| {
        app.run("modifier.handles", json!({})).unwrap()["handles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["handle"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let panel = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::appearance::show(app, ui));
        out.textures_delta.clear();
    };
    panel(&mut app);
    assert_eq!(names(&mut app), ["mirrorHorizontal"], "nothing open: the last row");
    crate::panels::set_pstate(&ctx, &format!("pc-open-{id}-m1"), true);
    panel(&mut app);
    assert_eq!(names(&mut app), ["linearEnd"], "the open row");
    crate::panels::set_pstate(&ctx, &format!("pc-open-{id}-m2"), true);
    panel(&mut app);
    assert_eq!(names(&mut app), ["linearEnd", "mirrorHorizontal"], "both open");
}

/// With `PICABIA_SHOTS` set to a folder: pictures of the handles on a linear array, a circular
/// array, a mirror and a deform row, beside the Appearance panel with the row open, light and
/// dark, the first handle hovered (its readout showing). Rendered offscreen (wgpu), so it works
/// with the screen locked.
#[test]
fn capture_handle_pictures() {
    use crate::theme::Brightness;
    let Some(dir) = std::env::var_os("PICABIA_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let scenes = [
        (
            "linear",
            json!({"x": 120, "y": 160, "width": 60, "height": 60}),
            json!({"kind": "linearArray", "props": {"count": 4, "offsetX": 90, "offsetY": 30}}),
        ),
        (
            "circular",
            json!({"x": 290, "y": 110, "width": 40, "height": 40}),
            json!({"kind": "circularArray", "props": {"count": 8, "radius": 110, "sweep": 270}}),
        ),
        ("mirror", json!({"x": 140, "y": 170, "width": 90, "height": 60}), json!({"kind": "mirror", "props": {"offset": 70}})),
        (
            "deform",
            json!({"x": 120, "y": 150, "width": 260, "height": 110}),
            json!({"kind": "deform", "props": {"mode": "taper", "startScale": 1, "endScale": 0.35}}),
        ),
    ];
    for (name, shape, row) in scenes {
        for brightness in [Brightness::Light, Brightness::Dark] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 560, "height": 420})).unwrap();
            let id = app.run("shape.rectangle", shape.clone()).unwrap()["id"].as_u64().unwrap();
            let mut add = row.clone();
            add["ids"] = json!([id]);
            app.run("modifier.add", add).unwrap();
            app.session.select_tool("selection", Default::default()).unwrap();
            app.ui.task_bar = false;
            app.ui.brightness = brightness;
            app.canvas.worker_started = true;
            let mut ready = false;
            let mut harness = egui_kittest::Harness::builder().with_size(vec2(1180.0, 700.0)).with_pixels_per_point(1.0).wgpu().build_ui_state(
                move |ui, app: &mut VectorcraftApp| {
                    if !ready {
                        egui_extras::install_image_loaders(ui.ctx());
                        crate::theme::install_fonts(ui.ctx());
                        crate::theme::apply(ui.ctx(), brightness);
                        crate::panels::set_pstate(ui.ctx(), &format!("pc-open-{id}-m1"), true);
                        ready = true;
                        return;
                    }
                    egui::Panel::right("pc-shot-appearance").exact_size(320.0).show(ui, |ui| crate::panels::appearance::show(app, ui));
                    show(app, ui);
                },
                app,
            );
            harness.input_mut().max_texture_side = Some(8192);
            harness.run_steps(4);
            let app = harness.state();
            let first = app.session.picabia_mod_handles(app.view_info()).and_then(|v| v.handles.first().map(|h| h.at));
            if let Some(at) = first.map(|p| xf(app).to_screen(p)) {
                harness.input_mut().events.push(egui::Event::PointerMoved(at));
            }
            harness.run_steps(4);
            let theme = if brightness == Brightness::Light { "light" } else { "dark" };
            harness.render().unwrap().save(dir.join(format!("c1-{name}-{theme}.png"))).unwrap();
        }
    }
}
