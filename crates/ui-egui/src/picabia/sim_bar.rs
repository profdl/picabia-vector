//! PICABIA: the physics transport bar and the session's clock (PIC31.5).
//!
//! While a physics session is open in the active document, a slim bar floats at the bottom of the
//! canvas: play/pause, reset, a scrubber with the time, Simulate to Rest, Apply and Cancel. Every
//! button is a `physics.*` command; the scrubber seeks. Each frame the playing session is stepped by
//! the frame's time (`session::tick`, at most four steps a frame, so a slow scene plays slower than
//! real time but stays exact) and the app keeps repainting. The canvas draws the session's clones
//! (the canvas seam asks [`display_doc`]).

use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::picabia_physics::session;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The bar's widest, points.
const BAR: egui::Vec2 = egui::Vec2::new(560.0, 40.0);
/// Steps a simulated second.
const HZ: f64 = 60.0;

/// The document the canvas draws (seam in `canvas::paint`): the session's clones over the dimmed
/// originals while a session is open, else `None` (the document itself).
pub fn display_doc(app: &VectorcraftApp) -> Option<Arc<Document>> {
    session::display_doc(&app.session)
}

/// Step the playing session by this frame's time and draw the bar (from `picabia_ui::show`).
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let dt = f64::from(ctx.input(|i| i.stable_dt));
    if session::tick(&mut app.session, dt) {
        ctx.request_repaint();
    }
    if let (Some(state), Some(canvas)) = (session::bar_state(&app.session), app.canvas_rect) {
        // Bottom centre of the canvas, as wide as it fits.
        let width = BAR.x.min(canvas.width() - 24.0).max(200.0);
        let at = pos2(canvas.center().x, canvas.bottom() - 12.0);
        egui::Area::new(Id::unique("picabia_physics_bar"))
            .order(egui::Order::Middle)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(at)
            .show(ctx, |ui| bar(app, ui, state, width));
    }
    pin_markers(app, ctx);
    // Playing (Play may have been pressed on the bar just now): keep the frames coming.
    if session::playing(&app.session) {
        ctx.request_repaint();
    }
}

/// What the bar asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Play,
    Pause,
    Reset,
    Seek(u32),
    ToRest,
    Apply,
    Cancel,
}

fn bar(app: &mut VectorcraftApp, ui: &mut Ui, state: session::BarState, width: f32) {
    let t = Tokens::get(ui.ctx());
    let mut action = None;
    egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 6)).show(
        ui,
        |ui| {
            ui.set_width(width - 16.0);
            // Narrow: the time and To Rest make room (the scrubber still shows the time).
            let narrow = width < 470.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (play, tip) = if state.playing { (false, tl!("Pause")) } else { (true, tl!("Play")) };
                if glyph_button(ui, if play { Glyph::Play } else { Glyph::Pause }, tip).clicked() {
                    action = Some(if play { Action::Play } else { Action::Pause });
                }
                if glyph_button(ui, Glyph::Reset, tl!("Back to the start")).clicked() {
                    action = Some(Action::Reset);
                }
                // The scrubber: every step reached so far (at least a second).
                let end = state.furthest.max(state.step).max(60);
                let mut step = state.step;
                let scrub = if narrow { (width - 240.0).clamp(40.0, 120.0) } else { (width - 390.0).clamp(60.0, 170.0) };
                let r = ui
                    .add_sized(vec2(scrub, 20.0), egui::Slider::new(&mut step, 0..=end).show_value(false).trailing_fill(true))
                    .on_hover_text(format!("{:.2} s", f64::from(state.step) / HZ));
                if r.changed() && step != state.step {
                    action = Some(Action::Seek(step));
                }
                let secs = f64::from(state.step) / HZ;
                if !narrow {
                    ui.label(egui::RichText::new(format!("{secs:.2} s")).monospace().size(11.5).color(if state.settled {
                        t.text_dim
                    } else {
                        t.text
                    }));
                    ui.add_space(4.0);
                    if crate::picabia_ui::button(ui, tl!("To Rest"), false, true) {
                        action = Some(Action::ToRest);
                    }
                }
                if crate::picabia_ui::button(ui, tl!("Apply"), true, true) {
                    action = Some(Action::Apply);
                }
                if crate::picabia_ui::button(ui, tl!("Cancel"), false, true) {
                    action = Some(Action::Cancel);
                }
            });
        },
    );
    if let Some(a) = action {
        run(app, a);
    }
}

fn run(app: &mut VectorcraftApp, a: Action) {
    let r = match a {
        Action::Play => app.run("physics.play", json!({})),
        Action::Pause => app.run("physics.pause", json!({})),
        Action::Reset => app.run("physics.reset", json!({})),
        Action::Seek(step) => app.run("physics.seek", json!({"step": step})),
        Action::ToRest => app.run("physics.toRest", json!({})),
        Action::Apply => app.run("physics.apply", json!({})),
        Action::Cancel => app.run("physics.cancel", json!({})),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Play,
    Pause,
    Reset,
}

/// A square button with a glyph drawn in code (no icon file).
fn glyph_button(ui: &mut Ui, g: Glyph, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let resp = resp.on_hover_text(tip);
    let col = if resp.hovered() { t.text_strong } else { t.icon };
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    paint_glyph(ui.painter(), r, g, col);
    resp
}

fn paint_glyph(p: &egui::Painter, r: Rect, g: Glyph, col: Color32) {
    let c = r.center();
    match g {
        Glyph::Play => {
            let pts = vec![pos2(c.x - 4.0, c.y - 6.0), pos2(c.x + 6.0, c.y), pos2(c.x - 4.0, c.y + 6.0)];
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
        Glyph::Pause => {
            p.rect_filled(Rect::from_center_size(pos2(c.x - 3.0, c.y), vec2(3.0, 12.0)), 1.0, col);
            p.rect_filled(Rect::from_center_size(pos2(c.x + 3.0, c.y), vec2(3.0, 12.0)), 1.0, col);
        }
        Glyph::Reset => {
            // A bar and a back-pointing triangle: to the start.
            p.rect_filled(Rect::from_center_size(pos2(c.x - 5.0, c.y), vec2(2.0, 12.0)), 0.5, col);
            let pts = vec![pos2(c.x + 5.0, c.y - 6.0), pos2(c.x - 3.0, c.y), pos2(c.x + 5.0, c.y + 6.0)];
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
    }
}

/// Pins on the canvas (PIC31.8): a ring where a point is pinned in place, a dot where it is attached
/// to another body; on the session's bodies where they are now, else on the selected soft objects.
fn pin_markers(app: &VectorcraftApp, ctx: &egui::Context) {
    let (Some(rect), Some(view)) = (app.canvas_rect, app.view()) else { return };
    let marks = vectorcraft_engine::cmd::picabia_physics::pins::markers(&app.session);
    if marks.is_empty() {
        return;
    }
    let xf = crate::canvas::Xf::at(rect, view, app.canvas_ui_zoom);
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, Id::unique("picabia_physics_pins"))).with_clip_rect(rect);
    for (p, attached) in marks.iter().take(512) {
        let at = xf.to_screen(*p);
        if !at.is_finite() {
            continue;
        }
        if *attached {
            painter.circle_filled(at, 3.5, t.selection);
        } else {
            painter.circle_stroke(at, 4.0, Stroke::new(1.5, t.selection));
        }
    }
}
