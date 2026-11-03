//! PICABIA: poking a simulated body with the Selection tool (PIC31.6), headless: a press on a
//! moving body grabs it and plays, a drag pulls it, a release lets go; elsewhere the tool gets the
//! input.

use egui::{Id, Pos2, Rect, Sense};
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use crate::VectorcraftApp;

const CANVAS: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(900.0, 700.0));

fn scene() -> (VectorcraftApp, egui::Context, u64) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let boxy = app.run("shape.rectangle", json!({"x": 180, "y": 200, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("physics.set", json!({"ids": [boxy]})).unwrap();
    app.run("select.set", json!({"ids": []})).unwrap();
    app.canvas_rect = Some(CANVAS);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    (app, ctx, boxy)
}

fn screen(app: &VectorcraftApp, p: Point) -> Pos2 {
    crate::canvas::Xf::new(CANVAS, app.view().unwrap()).to_screen(p)
}

/// One frame: the canvas's response, then the poke; whether the poke took the input.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> bool {
    frame_with(app, ctx, events, egui::Modifiers::default())
}

/// [`frame`] with modifier keys held.
fn frame_with(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> bool {
    let time = ctx.input(|i| i.time) + 1.0 / 60.0;
    let mut took = false;
    let mut events = events;
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| {
        let resp = ui.interact(CANVAS, Id::unique("canvas-test"), Sense::click_and_drag());
        took = super::sim_poke::poke(app, ui, &resp, CANVAS);
    });
    out.textures_delta.clear();
    // The app's clock (`sim_bar::show` steps a playing session each frame).
    vectorcraft_engine::cmd::picabia_physics::session::tick(&mut app.session, 1.0 / 60.0);
    took
}

fn button(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn a_drag_on_a_body_pulls_it_and_letting_go_throws_it() {
    let (mut app, ctx, boxy) = scene();
    app.run("physics.session.open", json!({})).unwrap();
    let centre = Point::new(200.0, 220.0);
    let at = screen(&app, centre);
    let to = screen(&app, Point::new(300.0, 80.0));
    assert!(!frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]));
    assert!(frame(&mut app, &ctx, vec![button(at, true)]), "the press grabs the box");
    assert!(app.picabia.poking);
    assert!(app.run("physics.session.info", json!({})).unwrap()["playing"].as_bool().unwrap(), "a grab plays");
    for k in 1..=30 {
        let p = at + (to - at) * (k as f32 / 30.0);
        assert!(frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]));
    }
    assert!(frame(&mut app, &ctx, vec![button(to, false)]));
    assert!(!app.picabia.poking, "let go");
    // The box went up and right, towards the pointer.
    let info = app.run("physics.session.info", json!({})).unwrap();
    assert!(info["events"].as_u64().unwrap() >= 3, "{info}");
    let id = NodeId(boxy);
    let poses = vectorcraft_engine::cmd::picabia_physics::session::bar_state(&app.session).unwrap();
    assert!(poses.step > 20, "{poses:?}");
    let now = vectorcraft_engine::cmd::picabia_physics::session::body_at(&app.session, centre);
    assert_ne!(now, Some(id), "the box is no longer where it was grabbed");
}

#[test]
fn a_press_elsewhere_or_without_a_session_reaches_the_tool() {
    let (mut app, ctx, _) = scene();
    let empty = screen(&app, Point::new(20.0, 20.0));
    assert!(!frame(&mut app, &ctx, vec![egui::Event::PointerMoved(empty), button(empty, true)]), "no session");
    frame(&mut app, &ctx, vec![button(empty, false)]);
    app.run("physics.session.open", json!({})).unwrap();
    assert!(!frame(&mut app, &ctx, vec![egui::Event::PointerMoved(empty), button(empty, true)]), "not on a body");
    frame(&mut app, &ctx, vec![button(empty, false)]);
    assert!(!app.picabia.poking);
}

#[test]
fn only_a_plain_primary_press_grabs_and_losing_the_drag_lets_go() {
    let (mut app, ctx, _) = scene();
    app.run("physics.session.open", json!({})).unwrap();
    let at = screen(&app, Point::new(200.0, 220.0));
    // A right click or a Shift-click on the body reaches the tool.
    let right = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() };
    assert!(!frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), right]));
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() }],
    );
    let shift = egui::Modifiers { shift: true, ..Default::default() };
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: shift };
    assert!(!frame_with(&mut app, &ctx, vec![press(true)], shift));
    frame_with(&mut app, &ctx, vec![press(false)], shift);
    assert!(!app.picabia.poking);
    // Grab, hold still (no more pulls recorded), then the session closes: let go.
    assert!(frame(&mut app, &ctx, vec![button(at, true)]));
    let n0 = app.run("physics.session.info", json!({})).unwrap()["events"].as_u64().unwrap();
    for _ in 0..10 {
        frame(&mut app, &ctx, vec![]);
    }
    let n1 = app.run("physics.session.info", json!({})).unwrap()["events"].as_u64().unwrap();
    assert_eq!(n0, n1, "holding still records nothing");
    app.run("physics.cancel", json!({})).unwrap();
    assert!(!frame(&mut app, &ctx, vec![]));
    assert!(!app.picabia.poking, "let go when the session closed");
}
