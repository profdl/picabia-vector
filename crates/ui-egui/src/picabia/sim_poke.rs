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
    let open = session::bar_state(&app.session).is_some() && app.session.tool_id() == "selection";
    if !open {
        // The session closed or the tool changed mid-drag: let go of what is held.
        if app.picabia.poking {
            release(app);
        }
        return false;
    }
    let Some(view) = app.view() else { return false };
    let xf = crate::canvas::Xf::at(rect, view, app.canvas_ui_zoom);
    let pointer: Option<Pos2> = ui.input(|i| i.pointer.interact_pos());
    if app.picabia.poking {
        // A pull only when the pointer moved (holding still records nothing more).
        if let Some(p) = pointer.filter(|p| app.picabia.poke_at != Some(*p)) {
            app.picabia.poke_at = Some(p);
            let to = xf.to_doc(p);
            // Refused only when the session's log is full: the body keeps its last pull.
            let _ = app.run("physics.poke", json!({"to": [to.x, to.y]}));
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            release(app);
        }
        return true;
    }
    // A plain primary press on a moving body: not another button, not with a modifier (Shift-click
    // and the rest stay the tool's), not with Space held (the Hand tool).
    let plain = ui.input(|i| i.pointer.primary_pressed() && i.modifiers.is_none() && !i.key_down(egui::Key::Space));
    if !(plain && resp.is_pointer_button_down_on()) {
        return false;
    }
    let Some(p) = pointer.filter(|p| rect.contains(*p)) else { return false };
    let at = xf.to_doc(p);
    let Some(id) = session::body_at(&app.session, at) else { return false };
    if app.run("physics.poke", json!({"id": id.0, "point": [at.x, at.y]})).is_ok() {
        app.picabia.poking = true;
        app.picabia.poke_at = Some(p);
        // A grab while paused plays, so the pull shows (playing a session that is open can't fail).
        let _ = app.run("physics.play", json!({}));
        return true;
    }
    false
}

/// Let go of the held body.
fn release(app: &mut VectorcraftApp) {
    app.picabia.poking = false;
    app.picabia.poke_at = None;
    // A release is never refused while the session is open (not even with a full log); without
    // one there is nothing held.
    let _ = app.run("physics.poke", json!({"release": true}));
}
