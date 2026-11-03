//! PICABIA: poking a simulated body with the Selection tool (PIC31.6).
//!
//! While a physics session is open, a Selection-tool drag that starts on a moving body (where it
//! is shown now: `session::body_at`) pokes it instead of reaching the tool: pressed, the body is
//! grabbed there (`physics.poke {id, point}`, a soft body by its nearest particle) and the session
//! plays; dragged, it is pulled towards the pointer by a spring (`physics.poke {to}`, each frame);
//! released, let go (`physics.poke {release}`), so it is thrown with the speed it has. Every step
//! is recorded at its step, so scrubbing replays it. A canvas seam calls [`poke`] before the tool.

use egui::{Pos2, Rect, Ui};
use serde_json::json;
use vectorcraft_engine::cmd::picabia_physics::session;

use crate::VectorcraftApp;

/// While a session is open, a Selection-tool drag that starts on a moving body pokes it (seam at the
/// top of `canvas::canvas_input`): pressed, the body is grabbed where it was pressed; dragged, it is
/// pulled towards the pointer; released, let go. `true` when the poke took the input (the tool
/// doesn't get it).
pub fn poke(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: Rect) -> bool {
    if session::bar_state(&app.session).is_none() || app.session.tool_id() != "selection" {
        app.picabia.poking = false;
        return false;
    }
    let Some(view) = app.view() else { return false };
    let xf = crate::canvas::Xf::new(rect, view);
    let pointer: Option<Pos2> = ui.input(|i| i.pointer.interact_pos());
    if app.picabia.poking {
        if let Some(p) = pointer {
            let to = xf.to_doc(p);
            // A pull the session refuses (its log is full) leaves the body where it is pulled to.
            let _ = app.run("physics.poke", json!({"to": [to.x, to.y]}));
        }
        if ui.input(|i| i.pointer.any_released()) || !resp.dragged() && !ui.input(|i| i.pointer.any_down()) {
            // Refused only when the session closed meanwhile: nothing is held then.
            let _ = app.run("physics.poke", json!({"release": true}));
            app.picabia.poking = false;
        }
        return true;
    }
    if !(resp.drag_started() || resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed())) {
        return false;
    }
    let Some(p) = pointer.filter(|p| rect.contains(*p)) else { return false };
    let at = xf.to_doc(p);
    let Some(id) = session::body_at(&app.session, at) else { return false };
    if app.run("physics.poke", json!({"id": id.0, "point": [at.x, at.y]})).is_ok() {
        app.picabia.poking = true;
        // A grab while paused plays, so the pull shows (playing a session that is open can't fail).
        let _ = app.run("physics.play", json!({}));
        return true;
    }
    false
}
