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
    set_pstate(&ctx, &format!("pc-open-{id}-m1"), true);
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
    assert_eq!(
        ids.iter().filter(|(id, _)| *id == "modifier.add").count(),
        super::appearance::KINDS.len(),
        "the same kinds as Add New Effect ▸ Modifiers"
    );
    for id in ["modifier.apply", "modifier.remove_all", "modifier.copy", "modifier.paste"] {
        assert!(ids.iter().any(|(i, _)| *i == id), "{id}");
        assert!(vectorcraft_engine::cmd::find_command(id).is_some());
    }
    let tree = crate::menus::menu_tree();
    let object = tree.iter().find(|(t, _)| *t == "Object").unwrap();
    assert!(object.1.iter().any(|i| matches!(i, crate::menus::Item::Sub("Modifiers", _))));
}

/// Two selected objects with the same kinds: one set of rows, and an edit reaches both.
#[test]
fn a_two_object_selection_edits_both_or_shows_mixed() {
    let (mut app, ctx, a) = arrayed();
    let b = app.run("shape.rectangle", json!({"x": 0, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("modifier.add", json!({"ids": [b], "kind": "linearArray", "props": {"count": 3}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s == "Linear Array"), "{shown:?}");
    // Turning the row off through the panel's command reaches both objects.
    set_pstate(&ctx, &format!("pc-open-{a}-m1"), true);
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Copies"));
    app.run("modifier.toggle", json!({"ids": [a, b], "index": 0, "enabled": false})).unwrap();
    let st = app.session.active().unwrap();
    for id in [a, b] {
        let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        assert!(!vectorcraft_modifiers::node_stack(n).unwrap().unwrap().modifiers[0].enabled);
    }
    // Different kinds: Mixed. The first selected object without a stack: Mixed too.
    app.run("modifier.add", json!({"ids": [b], "kind": "mirror"})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Modifiers: Mixed"));
    let plain = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [plain, b]})).unwrap();
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Modifiers: Mixed"));
}

/// A row's open state follows its modifier, not its position: moving the rows keeps it open.
#[test]
fn an_open_row_stays_open_when_rows_move_or_go() {
    let (mut app, ctx, id) = arrayed();
    app.run("modifier.add", json!({"ids": [id], "kind": "mirror", "index": 0})).unwrap();
    // The linear array (m1) is open, now the second row.
    set_pstate(&ctx, &format!("pc-open-{id}-m1"), true);
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Copies"));
    app.run("modifier.move", json!({"ids": [id], "from": 1, "to": 0})).unwrap();
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Copies"), "still open after the move");
    app.run("modifier.remove", json!({"ids": [id], "index": 1})).unwrap();
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Copies"), "still open after removing the mirror");
    assert!(!texts(&mut app, &ctx).iter().any(|s| s == "Axis"), "the mirror's fields never opened");
}

/// The panel says why an object draws once, first, as the canvas does.
#[test]
fn the_panel_says_when_an_object_draws_once() {
    let (mut app, ctx, _) = arrayed();
    let kids: Vec<u64> =
        (0..200).map(|i| app.run("shape.rectangle", json!({"x": i, "y": 300, "width": 1, "height": 1})).unwrap()["id"].as_u64().unwrap()).collect();
    let g = app.run("object.group", json!({"ids": kids})).unwrap()["id"].as_u64().unwrap();
    app.run("modifier.add", json!({"ids": [g], "kind": "linearArray", "props": {"count": 1000}})).unwrap();
    app.run("select.set", json!({"ids": [g]})).unwrap();
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s.contains("drawn once")), "{shown:?}");
}

/// Two objects whose stacks end in a stencil: one set of rows, and the stencil row asks for one
/// object instead of editing only the first.
#[test]
fn the_stencil_settings_edit_one_object() {
    let (mut app, ctx, a) = arrayed();
    let b = app.run("shape.rectangle", json!({"x": 0, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("modifier.add", json!({"ids": [b], "kind": "linearArray"})).unwrap();
    app.run("modifier.add", json!({"ids": [a, b], "kind": "stencil"})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let st = app.session.active().unwrap();
    let stencil = vectorcraft_modifiers::node_stack(st.doc.node(vectorcraft_doc::NodeId(a)).unwrap()).unwrap().unwrap().modifiers[1].id.clone();
    set_pstate(&ctx, &format!("pc-open-{a}-{stencil}"), true);
    assert!(texts(&mut app, &ctx).iter().any(|s| s == "Select one object to edit its stencil"));
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert!(!texts(&mut app, &ctx).iter().any(|s| s == "Select one object to edit its stencil"));
}

/// The geometry rows: their fields show by mode (tlprocedural's `isVisible`), fractions as
/// percentages, and a field edit reaches the stack clamped to the field's range.
#[test]
fn geometry_rows_show_the_fields_their_mode_uses() {
    let (mut app, ctx, a) = arrayed();
    for kind in ["resample", "trim", "tidy"] {
        app.run("modifier.add", json!({"ids": [a], "kind": kind})).unwrap();
    }
    for m in ["m2", "m3", "m4"] {
        set_pstate(&ctx, &format!("pc-open-{a}-{m}"), true);
    }
    let shown = texts(&mut app, &ctx);
    for want in ["Resample", "Trim", "Tidy", "Mode", "Points", "Start", "End", "Offset", "Close", "Join within", "Order", "Reverse"] {
        assert!(shown.iter().any(|s| s == want), "{want} in {shown:?}");
    }
    for hidden in ["Iterations", "Spacing", "Seed"] {
        assert!(!shown.iter().any(|s| s == hidden), "{hidden} hidden in {shown:?}");
    }
    app.run("modifier.set", json!({"ids": [a], "index": 1, "props": {"mode": "subdivide"}})).unwrap();
    app.run("modifier.set", json!({"ids": [a], "index": 3, "props": {"sort": "random"}})).unwrap();
    let shown = texts(&mut app, &ctx);
    for want in ["Iterations", "Method", "Seed"] {
        assert!(shown.iter().any(|s| s == want), "{want} in {shown:?}");
    }
    assert!(!shown.iter().any(|s| s == "Points"));
    // Trim's start reads as a percentage.
    let st = app.session.active().unwrap();
    let stack = vectorcraft_modifiers::node_stack(st.doc.node(vectorcraft_doc::NodeId(a)).unwrap()).unwrap().unwrap();
    assert_eq!(stack.modifiers[2].kind.params()["start"], 0.1);
    assert!(shown.iter().any(|s| s.starts_with("10")), "10% shown: {shown:?}");
}

/// The displace row (PIC20.4): spatial shows scale and octaves, radial the ripple count; the
/// progressive amount and reverse only while progressive is on.
#[test]
fn the_displace_row_shows_the_fields_its_mode_uses() {
    let (mut app, ctx, a) = arrayed();
    app.run("modifier.add", json!({"ids": [a], "kind": "displace"})).unwrap();
    set_pstate(&ctx, &format!("pc-open-{a}-m2"), true);
    let shown = texts(&mut app, &ctx);
    for want in ["Displace", "Mode", "Amplitude", "Scale", "Octaves", "Seed", "Ramp copies", "Ramp amount", "Ramp reversed"] {
        assert!(shown.iter().any(|s| s == want), "{want} in {shown:?}");
    }
    assert!(!shown.iter().any(|s| s == "Ripple count"), "{shown:?}");
    // Scale reads with two decimals.
    assert!(shown.iter().any(|s| s.starts_with("0.05")), "{shown:?}");
    app.run("modifier.set", json!({"ids": [a], "index": 1, "props": {"mode": "radial", "progressive": false}})).unwrap();
    let shown = texts(&mut app, &ctx);
    assert!(shown.iter().any(|s| s == "Ripple count"), "{shown:?}");
    for hidden in ["Scale", "Octaves", "Ramp amount", "Ramp reversed"] {
        assert!(!shown.iter().any(|s| s == hidden), "{hidden} hidden in {shown:?}");
    }
}

/// The deform row shows the fields its mode uses (tlprocedural's fields per deformer), and the
/// ramp's amount only with the ramp on.
#[test]
fn the_deform_row_shows_its_modes_fields() {
    let (mut app, ctx, a) = arrayed();
    app.run("modifier.add", json!({"ids": [a], "kind": "deform"})).unwrap();
    set_pstate(&ctx, &format!("pc-open-{a}-m2"), true);
    let shown = texts(&mut app, &ctx);
    for want in ["Deform", "Mode", "Axis", "Falloff", "Start scale", "End scale", "Ramp copies", "Ramp amount"] {
        assert!(shown.iter().any(|s| s == want), "{want} in {shown:?}");
    }
    for hidden in ["Angle", "Bend point", "Shear X", "Origin", "Strength", "Radius"] {
        assert!(!shown.iter().any(|s| s == hidden), "{hidden} hidden in {shown:?}");
    }
    app.run("modifier.set", json!({"ids": [a], "index": 1, "props": {"mode": "bulge", "progressive": false}})).unwrap();
    let shown = texts(&mut app, &ctx);
    for want in ["Strength", "Radius", "Falloff"] {
        assert!(shown.iter().any(|s| s == want), "{want} in {shown:?}");
    }
    for hidden in ["Axis", "Start scale", "Ramp amount"] {
        assert!(!shown.iter().any(|s| s == hidden), "{hidden} hidden in {shown:?}");
    }
    // Strength 0.35 reads as 35%.
    assert!(shown.iter().any(|s| s.starts_with("35")), "35% shown: {shown:?}");
}
