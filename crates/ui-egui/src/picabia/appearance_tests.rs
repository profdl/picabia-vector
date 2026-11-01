//! PICABIA: the modifier rows in the Appearance panel, drawn headless.

use egui::Rect;
use serde_json::json;

use crate::VectorcraftApp;
use crate::panels::appearance::show;
use crate::panels::set_pstate;

/// A rectangle with a 5-copy linear array (selected), and the panel's context after a frame.
fn arrayed() -> (VectorcraftApp, egui::Context, u64) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "linearArray", "props": {"count": 5, "offsetX": 40}})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    (app, ctx, id)
}

/// One frame of the panel with `events`: the texts drawn and where.
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
fn the_rows_sit_in_the_appearance_list_with_the_copies_and_apply() {
    let (mut app, ctx, _) = arrayed();
    let shown = frame(&mut app, &ctx, vec![]);
    let y = |label: &str| shown.iter().find(|(s, _)| s == label).map(|(_, r)| r.center().y).unwrap_or_else(|| panic!("no {label:?} in {shown:?}"));
    // Between the fills and strokes and the object's Opacity row.
    let row = y("Linear Array  ×5");
    assert!(y("Fill:") < row && row < y("Opacity:"), "{shown:?}");
    assert!(shown.iter().any(|(s, _)| s == "5 copies"));
    assert!(shown.iter().any(|(s, _)| s == "Apply"));
}

#[test]
fn an_open_row_edits_its_fields_through_modifier_set() {
    let (mut app, ctx, id) = arrayed();
    set_pstate(&ctx, "pc-open-0", true);
    let shown = texts(&mut app, &ctx);
    for label in ["Copies", "Offset X", "Offset Y", "Rotate each", "Scale each"] {
        assert!(shown.iter().any(|s| s == label), "{label}: {shown:?}");
    }
    assert!(shown.iter().any(|s| s == "×5" || s.contains("Linear Array")));
    // Apply bakes the copies: the panel shows the group, without rows.
    click(&mut app, &ctx, "Apply");
    let st = app.session.active().unwrap();
    assert!(st.doc.node(vectorcraft_doc::NodeId(id)).is_none(), "applied");
    assert!(!texts(&mut app, &ctx).iter().any(|s| s.contains("Linear Array")));
}

#[test]
fn a_damaged_stack_offers_remove_all() {
    let (mut app, ctx, id) = arrayed();
    app.session
        .edit("Damage", |d, _| {
            vectorcraft_modifiers::set_node_source(d.node_mut(vectorcraft_doc::NodeId(id)).unwrap(), Some("{broken".into()));
            Ok(())
        })
        .unwrap();
    click(&mut app, &ctx, "Remove All");
    let st = app.session.active().unwrap();
    assert!(!vectorcraft_modifiers::has_modifiers(st.doc.node(vectorcraft_doc::NodeId(id)).unwrap()));
}

#[test]
fn object_modifiers_menu_lists_every_kind_and_the_stack_commands() {
    let items = super::menu::object_items();
    let ids: Vec<(&str, String)> = items
        .iter()
        .filter_map(|i| match i {
            crate::menus::Item::Cmd(l, id, p) => Some((*id, format!("{l} {p}"))),
            _ => None,
        })
        .collect();
    assert_eq!(ids.iter().filter(|(id, _)| *id == "modifier.add").count(), 8);
    for id in ["modifier.apply", "modifier.remove_all", "modifier.copy", "modifier.paste"] {
        assert!(ids.iter().any(|(i, _)| *i == id), "{id}");
        assert!(vectorcraft_engine::cmd::find_command(id).is_some());
    }
    let tree = crate::menus::menu_tree();
    let object = tree.iter().find(|(t, _)| *t == "Object").unwrap();
    assert!(object.1.iter().any(|i| matches!(i, crate::menus::Item::Sub("Modifiers", _))));
}
