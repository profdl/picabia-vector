//! PICABIA (PIC3.4): where modifiers are found, drawn headless: the Properties panel's Modifiers
//! section, the task bar's Modifiers button, the canvas badge and the command palette.

use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::tests_removeanchors::{has, shapes_text};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64) -> u64 {
    app.run("shape.rectangle", json!({"x": x, "y": y, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap()
}

fn kinds(app: &VectorcraftApp, id: u64) -> Vec<String> {
    let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
    match vectorcraft_modifiers::node_stack(&n) {
        Some(Ok(s)) => s.modifiers.iter().map(|m| m.kind.id().to_string()).collect(),
        _ => vec![],
    }
}

fn undo_labels(app: &VectorcraftApp) -> Vec<String> {
    app.session.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    ctx
}

/// The texts `out` painted, with their rects.
fn texts_of(out: egui::FullOutput) -> Vec<(String, Rect)> {
    shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

/// One frame of the Properties panel.
fn props(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 1400.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::panels::properties::show(app, ui));
    out.textures_delta.clear();
    texts_of(out)
}

/// Press and release at `p` over frames drawn by `frame`, then one more frame → its texts.
fn click_at(p: Pos2, frame: &mut dyn FnMut(Vec<Event>) -> Vec<(String, Rect)>) -> Vec<(String, Rect)> {
    let b = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![Event::PointerMoved(p)]);
    frame(vec![b(true)]);
    frame(vec![b(false)]);
    frame(vec![])
}

/// The last place `label` was painted.
fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts.iter().rev().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
}

fn y(texts: &[(String, Rect)], label: &str) -> f32 {
    at(texts, label).y
}

/// A selected rectangle with a 6-copy linear array.
fn arrayed() -> (VectorcraftApp, u64) {
    let mut app = app();
    let id = rect(&mut app, 40.0, 40.0);
    app.run("modifier.add", json!({"ids": [id], "kind": "linearArray", "props": {"count": 6, "offsetX": 10}})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, id)
}

#[test]
fn properties_shows_the_modifiers_section_between_appearance_and_align() {
    let (mut app, id) = arrayed();
    let ctx = ctx();
    props(&mut app, &ctx, vec![]);
    let texts = props(&mut app, &ctx, vec![]);
    for label in ["Modifiers", "Linear Array  ×6", "6 copies", "Apply", "+ Add Modifier"] {
        assert!(has(&texts, label), "{label}: {texts:?}");
    }
    assert!(y(&texts, "Appearance") < y(&texts, "Modifiers") && y(&texts, "+ Add Modifier") < y(&texts, "Align"), "{texts:?}");
    // The row's disclosure opens its fields (kept with the Appearance panel's rows).
    crate::panels::set_pstate(&ctx, &format!("pc-open-{id}-m1"), true);
    let texts = props(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Copies") && has(&texts, "Offset X"), "{texts:?}");
    crate::panels::set_pstate(&ctx, &format!("pc-open-{id}-m1"), false);
    // The header collapses the section, and opens it again.
    let texts = click_at(at(&texts, "Modifiers"), &mut |e| props(&mut app, &ctx, e));
    assert!(app.picabia.modifiers_collapsed && has(&texts, "Modifiers") && !has(&texts, "Linear Array  ×6") && !has(&texts, "+ Add Modifier"));
    click_at(at(&texts, "Modifiers"), &mut |e| props(&mut app, &ctx, e));
    assert!(!app.picabia.modifiers_collapsed);
}

#[test]
fn properties_shows_the_section_for_a_plain_object_and_not_without_one() {
    let mut app = app();
    let id = rect(&mut app, 40.0, 40.0);
    let ctx = ctx();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let texts = props(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Modifiers") && has(&texts, "+ Add Modifier") && !has(&texts, "Apply"), "{texts:?}");
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(!has(&props(&mut app, &ctx, vec![]), "Modifiers"));
    // Two objects whose stacks differ: one Mixed row, as in the Appearance panel.
    let b = rect(&mut app, 140.0, 40.0);
    app.run("modifier.add", json!({"ids": [id], "kind": "mirror"})).unwrap();
    app.run("select.set", json!({"ids": [id, b]})).unwrap();
    assert!(has(&props(&mut app, &ctx, vec![]), "Modifiers: Mixed"));
}

#[test]
fn add_modifier_runs_modifier_add_with_the_kind_or_preset_in_one_undo_step() {
    for (label, kind) in [("Circular Array", "circularArray"), ("Ben-Day Dots", "screen"), ("Stencil", "stencil")] {
        let mut app = app();
        let id = rect(&mut app, 40.0, 40.0);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let ctx = ctx();
        let texts = props(&mut app, &ctx, vec![]);
        let before = undo_labels(&app);
        let menu = click_at(at(&texts, "+ Add Modifier"), &mut |e| props(&mut app, &ctx, e));
        for group in ["Repeat", "Shape", "Make", "Physics"] {
            assert!(has(&menu, group), "{group}: {menu:?}");
        }
        click_at(at(&menu, label), &mut |e| props(&mut app, &ctx, e));
        assert_eq!(kinds(&app, id), [kind], "{label}");
        let after = undo_labels(&app);
        assert_eq!((after.len(), after.last().map(String::as_str)), (before.len() + 1, Some("Add Modifier")), "{label}: one undo step");
        if label == "Ben-Day Dots" {
            let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().clone();
            let p = vectorcraft_modifiers::node_stack(&n).unwrap().unwrap().modifiers[0].kind.params();
            assert_eq!((p["spot"].as_str(), p["tone"].as_str()), (Some("round"), Some("fixed")), "the preset");
        }
        app.run("edit.undo", json!({})).unwrap();
        assert!(kinds(&app, id).is_empty());
    }
}

/// Every kind and preset the modifiers crate and the menus know is in the grouped menu once, and
/// Object ▸ Modifiers has the same entries.
#[test]
fn the_grouped_menu_has_every_kind_and_preset() {
    let entries: Vec<_> = super::add_menu::GROUPS.iter().flat_map(|(_, e)| e.iter()).collect();
    for (kind, _) in vectorcraft_modifiers::KINDS {
        assert!(entries.iter().any(|e| e.kind == *kind && e.preset.is_none()), "{kind}");
    }
    for (kind, preset, _) in super::appearance::PRESETS {
        assert!(entries.iter().any(|e| e.kind == *kind && e.preset == Some(*preset)), "{preset}");
    }
    assert_eq!(entries.len(), vectorcraft_modifiers::KINDS.len() + super::appearance::PRESETS.len());
    let labels: Vec<&str> = super::menu::object_items()
        .iter()
        .filter_map(|i| match i {
            crate::menus::Item::Cmd(l, "modifier.add", _) => Some(*l),
            _ => None,
        })
        .collect();
    assert_eq!(labels, entries.iter().map(|e| e.label).collect::<Vec<_>>());
}

/// What one canvas frame drew: its texts and the controls screen readers name, with their rects.
struct Frame {
    texts: Vec<(String, Rect)>,
    named: Vec<(String, Rect)>,
}

/// One canvas frame `width` × 700 (the canvas, then what Picabia draws over the app), with an area
/// `cover` drawn over the canvas first when given (a floating panel's stand-in).
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, width: f32, cover: Option<Rect>) -> Frame {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
        crate::canvas::show(app, ui);
        if let Some(c) = cover {
            egui::Area::new(egui::Id::unique("test-cover")).order(egui::Order::Middle).fixed_pos(c.min).show(ui.ctx(), |ui| {
                ui.allocate_exact_size(c.size(), egui::Sense::click());
            });
        }
        crate::picabia_ui::show(app, ui.ctx());
    });
    out.textures_delta.clear();
    let named = out
        .platform_output
        .accesskit_update
        .as_ref()
        .map(|u| {
            u.nodes
                .iter()
                .filter_map(|(_, n)| {
                    let b = n.bounds()?;
                    Some((n.label()?.to_string(), Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32))))
                })
                .collect()
        })
        .unwrap_or_default();
    Frame { texts: texts_of(out), named }
}

/// One canvas frame at 1000 × 700 → its texts.
fn canvas(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    frame(app, ctx, events, 1000.0, None).texts
}

/// The task bar's Modifiers button, found by its accessible name (none while it isn't drawn).
fn modifiers_button(f: &Frame) -> Option<Pos2> {
    f.named.iter().find(|(l, _)| l == "Modifiers").map(|(_, r)| r.center())
}

#[test]
fn the_task_bar_button_opens_the_add_menu() {
    let mut app = app();
    let id = rect(&mut app, 60.0, 60.0);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let ctx = ctx();
    canvas(&mut app, &ctx, vec![]);
    let f = frame(&mut app, &ctx, vec![], 1000.0, None);
    let menu = click_at(modifiers_button(&f).expect("the button"), &mut |e| canvas(&mut app, &ctx, e));
    assert!(has(&menu, "Linear Array") && has(&menu, "Line Fill") && !has(&menu, "Show Modifiers"), "{menu:?}");
    click_at(at(&menu, "Grid Array"), &mut |e| canvas(&mut app, &ctx, e));
    assert_eq!(kinds(&app, id), ["gridArray"]);
    assert_eq!(undo_labels(&app).last().map(String::as_str), Some("Add Modifier"));
    // With a stack: Show Modifiers, Apply and Remove All first. Show Modifiers opens the
    // Properties panel's section.
    let f = frame(&mut app, &ctx, vec![], 1000.0, None);
    let menu = click_at(modifiers_button(&f).expect("the button"), &mut |e| canvas(&mut app, &ctx, e));
    for label in ["Show Modifiers", "Apply Modifiers", "Remove All Modifiers"] {
        assert!(has(&menu, label), "{label}: {menu:?}");
    }
    app.picabia.modifiers_collapsed = true;
    click_at(at(&menu, "Show Modifiers"), &mut |e| canvas(&mut app, &ctx, e));
    assert!(app.picabia.reveal_modifiers && !app.picabia.modifiers_collapsed);
    assert!(crate::panel_docking::showing(&app.ui, "properties"));
}

/// The bar keeps the button while it fits the canvas with it, and drops it (only it) in a window
/// too narrow for it, without coming and going from frame to frame.
#[test]
fn the_task_bar_drops_the_button_when_the_bar_would_not_fit() {
    let mut app = app();
    let id = rect(&mut app, 60.0, 60.0);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let ctx = ctx();
    canvas(&mut app, &ctx, vec![]);
    for _ in 0..3 {
        let f = frame(&mut app, &ctx, vec![], 1000.0, None);
        assert!(modifiers_button(&f).is_some(), "a wide window has it");
    }
    let canvas_w = |app: &VectorcraftApp| app.canvas_rect.unwrap().width();
    // Narrow the window until the bar no longer has room for the button.
    let mut dropped = None;
    for w in (300..1000).rev().step_by(20) {
        let a = frame(&mut app, &ctx, vec![], w as f32, None);
        let b = frame(&mut app, &ctx, vec![], w as f32, None);
        assert_eq!(modifiers_button(&a).is_some(), modifiers_button(&b).is_some(), "steady at {w}");
        if modifiers_button(&b).is_none() {
            dropped = Some((w, b));
            break;
        }
    }
    let (w, f) = dropped.expect("a narrow window drops it");
    assert!(has(&f.texts, "Duplicate"), "the rest of the bar stays at {w}: {:?}", f.texts);
    let bar = crate::canvas::task_bar_rect(&ctx).expect("the bar shows");
    assert!(bar.width() <= canvas_w(&app), "{bar:?} in {}", canvas_w(&app));
}

/// Where badge `i` was drawn (none while it isn't shown).
fn badge(ctx: &egui::Context, i: usize) -> Option<Rect> {
    let id = super::badge::area_id(i);
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// A selected rectangle with two modifiers, its badge drawn → the app, its context and the badge.
fn badged() -> (VectorcraftApp, egui::Context, u64, Rect) {
    let mut app = app();
    app.ui.task_bar = false;
    let id = rect(&mut app, 60.0, 60.0);
    app.run("modifier.add", json!({"ids": [id], "kind": "mirror"})).unwrap();
    app.run("modifier.add", json!({"ids": [id], "kind": "trim"})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let ctx = ctx();
    canvas(&mut app, &ctx, vec![]);
    canvas(&mut app, &ctx, vec![]);
    let r = badge(&ctx, 0).expect("a badge for the stacked object");
    (app, ctx, id, r)
}

#[test]
fn the_badge_shows_for_stacked_selected_objects_and_opens_the_section() {
    let mut app = app();
    app.ui.task_bar = false;
    let plain = rect(&mut app, 60.0, 60.0);
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    let ctx = ctx();
    canvas(&mut app, &ctx, vec![]);
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_none(), "no badge without modifiers");
    let (mut app, ctx, id, r) = badged();
    let texts = canvas(&mut app, &ctx, vec![]);
    assert!(has(&texts, "2"), "the number of rows: {texts:?}");
    // Up and right of the bounding box's top-right corner, clear of its handle.
    let (Some(view), Some(area)) = (app.view(), app.canvas_rect) else { panic!("a view") };
    let xf = crate::canvas::Xf::new(area, view);
    let b = app.session.active().unwrap().doc.bounds_of(&[NodeId(id)], true).unwrap();
    let corner = xf.to_screen(vectorcraft_geom::Point::new(b.x1, b.y0));
    assert!(r.left() > corner.x + 5.0 && r.bottom() < corner.y - 5.0, "{r:?} by {corner:?}");
    // A click opens the Properties panel's Modifiers section, and leaves the selection alone.
    app.picabia.modifiers_collapsed = true;
    let undo = undo_labels(&app).len();
    click_at(r.center(), &mut |e| canvas(&mut app, &ctx, e));
    assert!(app.picabia.reveal_modifiers && !app.picabia.modifiers_collapsed);
    assert!(crate::panel_docking::showing(&app.ui, "properties"));
    assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(id)], "the selection is unchanged");
    assert_eq!(undo_labels(&app).len(), undo, "no edit");
    // Not with a drawing tool, nor once the selection is gone.
    app.select_tool("rectangle");
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_none(), "not with the Rectangle tool");
    app.select_tool("selection");
    app.run("select.set", json!({"ids": []})).unwrap();
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_none(), "not without a selection");
}

/// The badge follows the task bar's conditions (no badge in Presentation Mode) and gives way to a
/// floating panel over it, which then gets the click.
#[test]
fn the_badge_hides_in_presentation_mode_and_under_a_floating_panel() {
    let (mut app, ctx, _, r) = badged();
    app.ui.screen_mode = 3;
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_none(), "not in Presentation Mode");
    app.ui.screen_mode = 0;
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_some());
    let cover = Some(Rect::from_center_size(r.center(), vec2(200.0, 120.0)));
    for _ in 0..2 {
        frame(&mut app, &ctx, vec![], 1000.0, cover);
    }
    assert!(badge(&ctx, 0).is_none(), "not under a floating panel");
    app.picabia.reveal_modifiers = false;
    click_at(r.center(), &mut |e| frame(&mut app, &ctx, e, 1000.0, cover).texts);
    assert!(!app.picabia.reveal_modifiers, "the panel gets the click");
    canvas(&mut app, &ctx, vec![]);
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_some(), "back once the panel is gone");
}

/// Show Modifiers and the badge leave the Properties panel open wherever it is (docked, a flyout,
/// a floating window) and open its section: they never put it away.
#[test]
fn reveal_keeps_the_properties_panel_open_wherever_it_is() {
    let (mut app, ctx, _, _) = badged();
    let shown = |app: &VectorcraftApp| crate::panel_docking::showing(&app.ui, "properties");
    let reveal_twice = |app: &mut VectorcraftApp, ctx: &egui::Context, place: &str| {
        for _ in 0..2 {
            app.picabia.modifiers_collapsed = true;
            super::properties::reveal(app);
            assert!(shown(app), "{place}: still open");
            assert!(app.picabia.reveal_modifiers && !app.picabia.modifiers_collapsed, "{place}");
            // The section's next frame scrolls to it and takes the request.
            props(app, ctx, vec![]);
            assert!(!app.picabia.reveal_modifiers, "{place}: done");
        }
    };
    // Docked (the Essentials workspace).
    assert!(shown(&app));
    reveal_twice(&mut app, &ctx, "docked");
    // A flyout from the collapsed dock.
    app.run("window.collapseDock", json!({"collapsed": true})).unwrap();
    app.run("window.panel", json!({"panel": "properties"})).unwrap();
    assert!(shown(&app), "the flyout shows");
    reveal_twice(&mut app, &ctx, "flyout");
    app.run("window.collapseDock", json!({"collapsed": false})).unwrap();
    // Floating.
    app.run("window.panel.float", json!({"panel": "properties"})).unwrap();
    app.run("window.panel.activate", json!({"panel": "properties"})).unwrap();
    assert!(shown(&app), "the window shows");
    reveal_twice(&mut app, &ctx, "floating");
    // Closed: it opens.
    app.run("window.panel.close", json!({"panel": "properties"})).unwrap();
    assert!(!shown(&app));
    super::properties::reveal(&mut app);
    assert!(shown(&app), "opened");
}

#[test]
fn badges_go_on_up_to_eight_stacked_objects() {
    let mut app = app();
    app.ui.task_bar = false;
    let ids: Vec<u64> = (0..9).map(|i| rect(&mut app, 10.0 + 42.0 * i as f64, 150.0)).collect();
    app.run("modifier.add", json!({"ids": ids, "kind": "mirror"})).unwrap();
    let ctx = ctx();
    app.run("select.set", json!({"ids": &ids[..2]})).unwrap();
    canvas(&mut app, &ctx, vec![]);
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_some() && badge(&ctx, 1).is_some() && badge(&ctx, 2).is_none());
    app.run("select.set", json!({"ids": ids})).unwrap();
    canvas(&mut app, &ctx, vec![]);
    canvas(&mut app, &ctx, vec![]);
    assert!(badge(&ctx, 0).is_none(), "none past eight");
}

/// The palette finds each kind and preset by name (one command each, `modifier.add.<kind>`),
/// among the results it shows, and a pick adds it to the selection.
#[test]
fn the_palette_finds_each_kind_and_preset_by_name() {
    let items = crate::palette::items();
    let shown = |q: &str| -> Vec<String> {
        let q = q.to_lowercase();
        items
            .iter()
            .filter(|(label, id, _)| {
                let hay = format!("{}\n{}", label.to_lowercase(), id.to_lowercase());
                q.split_whitespace().all(|w| hay.contains(w))
            })
            .take(14)
            .map(|(_, id, _)| id.clone())
            .collect()
    };
    for (query, id) in [
        ("Linear Array", "modifier.add.linearArray"),
        ("Add Circular Array", "modifier.add.circularArray"),
        ("Ben-Day Dots", "modifier.add.benDay"),
        ("Line Fill", "modifier.add.lineFill"),
        ("Screen", "modifier.add.screen"),
        ("Displace", "modifier.add.displace"),
        ("Stencil", "modifier.add.stencil"),
        ("Simulate Physics", "physics.simulate"),
    ] {
        assert!(shown(query).iter().any(|s| s == id), "{query:?} finds {id}: {:?}", shown(query));
    }
    for group in super::add_menu::GROUPS {
        for e in group.1 {
            let id = format!("modifier.add.{}", e.preset.unwrap_or(e.kind));
            assert!(items.iter().any(|(label, i, _)| *i == id && label.ends_with(e.label)), "{id}");
        }
    }
    // A pick runs it as the palette does: no params, the selection.
    let mut app = app();
    let id = rect(&mut app, 40.0, 40.0);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    crate::menus::invoke(&mut app, "modifier.add.circularArray", json!({}));
    assert_eq!(kinds(&app, id), ["circularArray"]);
}
