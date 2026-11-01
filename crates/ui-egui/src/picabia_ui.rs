//! PICABIA: what Picabia's laser and stencil UI shares, and its few ties to the app: its state in
//! [`crate::VectorcraftApp::picabia`], its UI commands (`laser.panel`, `ui.stencilSettings`), the
//! dialog a laser export command asks for when it is run without a path, what it draws each frame
//! (the Laser window, its bar and checklist, the Stencil window and the stencil's canvas markers)
//! and small widgets in VectorCraft's look. Upstream files reach these through one-line
//! `// PICABIA:` seams (`docs/picabia/SEAMS.md`).

use egui::{Color32, Ui};
use serde_json::Value;

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// UI command: open the Laser window (File ▸ Laser…).
pub const LASER_PANEL: &str = "laser.panel";
/// UI command: open the Stencil window (Window ▸ Stencil Settings…).
pub const STENCIL_SETTINGS: &str = "ui.stencilSettings";

/// Picabia's UI state (not the document).
#[derive(Default)]
pub struct PicabiaUi {
    /// The Laser window.
    pub laser: crate::picabia_laser_window::LaserPanel,
    /// The Stencil window is open.
    pub stencil_open: bool,
}

/// Picabia's UI commands (seam in `menus::run_ui_command`); `None`: not one of them.
pub fn command(app: &mut VectorcraftApp, id: &str, _p: &Value) -> Option<Result<Value, String>> {
    match id {
        LASER_PANEL => {
            crate::picabia_laser_window::open(app);
            Some(Ok(serde_json::json!({"open": true})))
        }
        STENCIL_SETTINGS => {
            app.picabia.stencil_open = true;
            Some(Ok(serde_json::json!({"open": true})))
        }
        _ => None,
    }
}

/// After engine command `id` returned `v` (seam in `run_now`): a laser export run without a path
/// asks for its dialog (`{dialog: "stencilExport" | "stencilTestCard" | "laserGcode", …}`).
pub fn after_command(app: &mut VectorcraftApp, id: &str, v: &Value) {
    crate::picabia_laser_export::open_requested(app, id, v);
}

/// Everything Picabia draws over the app each frame (seam in `ui_frame`, after the progress
/// windows): the stencil's canvas markers, the Stencil window, the Laser window (or its bar while
/// connected and closed) and the safety checklist.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    crate::picabia_stencil_panel::canvas_markers(app, ctx);
    crate::picabia_stencil_panel::window(app, ctx);
    crate::picabia_laser_window::show(app, ctx);
}

/// A button in the app's look: the accent one when `primary`; greyed and not clickable unless
/// `enabled`. Whether it was clicked.
pub(crate) fn button(ui: &mut Ui, text: &str, primary: bool, enabled: bool) -> bool {
    if primary {
        return widgets::primary_button_enabled(ui, text, enabled).clicked();
    }
    ui.add_enabled_ui(enabled, |ui| widgets::secondary_button(ui, text)).inner.clicked() && enabled
}

/// A line of text in `color` that wraps to the width it is given.
pub(crate) fn wrapped(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(color)).wrap());
}

/// A dim one-line label (not translated: the caller has).
pub(crate) fn note(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).size(11.5).color(t.text_dim));
}

/// `c` as text on the panel: the theme's warning yellow is hard to read on a light panel, so text
/// in it moves part way to the text colour there.
pub(crate) fn readable(c: Color32, t: &Tokens) -> Color32 {
    if !t.dark && c == t.warning { c.lerp_to_gamma(t.text, 0.55) } else { c }
}

/// `v` in mm or a count as the UI shows it: at most 2 decimals, no trailing zeros.
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The string `v` holds under `k`.
pub(crate) fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// The finite number `v` holds under `k`.
pub(crate) fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// The strings of the list `v` holds under `k`, at most `cap`.
pub(crate) fn strings(v: &Value, k: &str, cap: usize) -> Vec<String> {
    v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(cap).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_short() {
        assert_eq!(num(230.1), "230.1");
        assert_eq!(num(12.0), "12");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(0.126), "0.13");
    }

    #[test]
    fn ui_commands_open_the_windows() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert!(app.run(LASER_PANEL, serde_json::json!({})).is_ok());
        assert!(app.picabia.laser.open);
        assert!(app.run(STENCIL_SETTINGS, serde_json::json!({})).is_ok());
        assert!(app.picabia.stencil_open);
    }
}
