//! PICABIA: the stencil section and window, headless.

use serde_json::json;

use super::*;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app
}

/// A rectangle with a white ellipse in it, made a stencil (computed now) → its id.
fn stencil(app: &mut VectorcraftApp) -> NodeId {
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 200, "height": 120})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.ellipse", json!({"x": 60, "y": 40, "width": 80, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [a], "color": "#202020"})).unwrap();
    app.run("paint.setFill", json!({"ids": [b], "color": "#ffffff"})).unwrap();
    let g = app.run("modifier.stencilFromSelection", json!({"ids": [a, b], "props": {"widthMm": 200}})).unwrap()["id"].as_u64().unwrap();
    // Computes it now (the app's background refresh would, a moment later).
    app.run("modifier.stencilInfo", json!({"id": g})).unwrap();
    NodeId(g)
}

fn texts(app: &mut VectorcraftApp, dark: bool, mut draw: impl FnMut(&mut VectorcraftApp, &mut Ui)) -> String {
    let ctx = crate::widgets::look_tests::themed(if dark { crate::theme::Brightness::Dark } else { crate::theme::Brightness::Light });
    let mut text = String::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| draw(app, ui));
        });
        out.textures_delta.clear();
        text.clear();
        for s in out.shapes {
            if let egui::Shape::Text(t) = s.shape {
                text.push_str(t.galley.text());
                text.push('\n');
            }
        }
    }
    text
}

#[test]
fn the_stencil_row_is_the_first_enabled_one_and_knows_the_repeat_above() {
    let list = json!({"modifiers": [
        {"id": "m1", "kind": "wallpaper", "enabled": true},
        {"id": "m2", "kind": "stencil", "enabled": false},
        {"id": "m3", "kind": "stencil", "enabled": true, "widthMm": 120},
    ]});
    let r = stencil_row(&list).unwrap();
    assert_eq!((r.id.as_str(), r.lattice_above, r.num("widthMm", 0.0)), ("m3", true, 120.0));
    assert_eq!(stencil_row(&json!({"modifiers": [{"id": "m1", "kind": "mirror"}]})), None);
    assert_eq!(stencil_row(&json!(null)), None);
    assert_eq!(stencil_row(&json!({"modifiers": [{"kind": "stencil"}]})), None, "a row without an id");
}

#[test]
fn stats_and_problems_read_as_plain_words() {
    let info = Info::from_json(
        &json!({"stats": {"cutPieces": 3, "droppedSpecks": 1, "cutLengthMm": 812.5, "islands": 2, "ties": 4, "untied": 1, "patternHeld": 0,
            "problems": {"narrowPaper": 12, "sharpCorner": 1}},
            "problems": [{"kind": "narrowPaper", "x": 1, "y": 2, "w": 3, "h": 4}, {"kind": "junk"}, 5]}),
        true,
    );
    assert_eq!(stats_line(&info, false), "3 pieces, 1 island tied with 4 ties, 1 speck dropped, 812.5 mm cut");
    assert_eq!(stats_line(&info, true), "3 pieces, 2 islands held by the gauze, 1 speck dropped, 812.5 mm cut");
    let rows: Vec<(&str, u64)> = problem_list(&info, false).iter().map(|(k, .., n)| (*k, *n)).collect();
    assert_eq!(rows, [("narrowPaper", 12), ("sharpCorner", 1), ("untied", 1)]);
    assert!(problem_list(&info, true).iter().all(|(k, ..)| *k != "untied"), "gauze holds them");
    assert_eq!(info.problems, [("narrowPaper".to_string(), Rect::new(1.0, 2.0, 4.0, 6.0))]);
    assert_eq!(Info::from_json(&json!("junk"), false), Info::default());
}

#[test]
fn the_section_shows_every_group_and_the_result_in_both_themes() {
    let mut app = app();
    let g = stencil(&mut app);
    for dark in [false, true] {
        let text = texts(&mut app, dark, |app, ui| stencil_section(ui, app, g));
        for want in
            ["Colours", "Size", "Clean-up", "Ties", "Repeat", "Preview", "Width", "Sheet", "pieces", "mm cut", "Export for Laser…", "Test Card…"]
        {
            assert!(text.contains(want), "{want} in {text}");
        }
    }
}

#[test]
fn an_edit_is_one_modifier_set_and_the_buttons_open_the_laser_dialogs() {
    let mut app = app();
    let g = stencil(&mut app);
    let list = app.session.execute("modifier.list", &json!({"id": g.0})).unwrap();
    let row = stencil_row(&list).unwrap();
    set(&mut app, g, &row, json!({"tieMode": "gauze"}));
    let row = stencil_row(&app.session.execute("modifier.list", &json!({"id": g.0})).unwrap()).unwrap();
    assert_eq!(row.str("tieMode", ""), "gauze");
    app.run("modifier.stencilExport", json!({"id": g.0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::picabia_laser_export::EXPORT));
}

#[test]
fn an_object_without_a_stencil_offers_to_make_one_and_the_window_follows_the_selection() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let text = texts(&mut app, false, |app, ui| stencil_section(ui, app, NodeId(id)));
    assert!(text.contains("Make Stencil"), "{text}");
    app.run(crate::picabia_ui::STENCIL_SETTINGS, json!({})).unwrap();
    app.run("select.set", json!({"ids": []})).unwrap();
    let ctx = crate::widgets::look_tests::themed(crate::theme::Brightness::Dark);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| window(&mut app, ui.ctx()));
    out.textures_delta.clear();
    // The second frame: a new window lays itself out unseen first.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| window(&mut app, ui.ctx()));
    out.textures_delta.clear();
    let shown: String =
        out.shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t.galley.text().to_string()) } else { None }).collect();
    assert!(shown.contains("Select a stencil object"), "{shown}");
    assert!(app.picabia.stencil_open);
}

#[test]
fn a_problem_row_click_centres_the_view_on_the_place() {
    let mut app = app();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let before = app.view().map(|v| v.center);
    reveal(&mut app, Rect::new(100.0, 100.0, 104.0, 104.0));
    let v = app.view().unwrap();
    assert_eq!(v.center, Point::new(102.0, 102.0));
    assert_ne!(Some(v.center), before);
    reveal(&mut app, Rect::new(f64::NAN, 0.0, 1.0, 1.0));
    assert_eq!(app.view().unwrap().center, Point::new(102.0, 102.0), "junk leaves the view");
}
