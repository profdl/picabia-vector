//! PICABIA: what Picabia's laser, stencil and camera UI shares, and its few ties to the app: its state in
//! [`crate::VectorcraftApp::picabia`], its UI commands (`laser.panel`, `ui.stencilSettings`, the
//! camera's in `picabia_camera_window`), the
//! dialog a laser export command asks for when it is run without a path, what it draws each frame
//! (the Laser window, its bar and checklist, the Stencil window and the stencil's canvas markers)
//! and small widgets in VectorCraft's look. Upstream files reach these through one-line
//! `// PICABIA:` seams (`docs/picabia/SEAMS.md`).

use egui::{Color32, Ui};
use serde_json::Value;

use crate::VectorcraftApp;

pub(crate) mod widgets;

/// The colours Picabia's UI draws with, read from the framework's resolved theme tokens (every theme
/// applies), under the names the Picabia code used before VectorCraft moved onto the framework.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub panel: Color32,
    pub panel_darker: Color32,
    pub input: Color32,
    pub input_border: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub text_strong: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub accent: Color32,
    pub row_selected: Color32,
    pub selection: Color32,
    pub button: Color32,
    pub measure_bg: Color32,
    pub radius: u8,
    pub error: Color32,
    pub warning: Color32,
}

impl Tokens {
    /// The tokens of the window `ctx` draws (its root theme).
    pub fn get(ctx: &egui::Context) -> Self {
        Self::of(craft::ui::runtime::env::Env::of_root(ctx).tokens.tokens())
    }

    fn of(t: &craft::theme::Tokens) -> Self {
        let c = craft::ui::style::rgba_color;
        let k = &t.color;
        let panel = c(k.surface.panel);
        Tokens {
            dark: panel.r() as u32 + panel.g() as u32 + panel.b() as u32 <= 3 * 128,
            panel,
            panel_darker: c(k.surface.sunken),
            input: c(k.surface.sunken),
            input_border: c(k.line.control_edge),
            border: c(k.line.border),
            text: c(k.text.normal),
            text_dim: c(k.text.muted),
            text_disabled: c(k.text.disabled),
            text_strong: c(k.text.normal),
            icon: c(k.text.normal),
            hover: c(k.state.hover),
            accent: c(k.accent.solid),
            row_selected: c(k.state.selected),
            selection: c(k.overlay.stroke),
            button: c(k.surface.control),
            measure_bg: c(k.surface.raised),
            radius: 4,
            error: c(k.status.bad.fg),
            warning: c(k.status.warn.fg),
        }
    }
}

/// UI command: open the Laser window (File ▸ Laser…).
pub const LASER_PANEL: &str = "laser.panel";
/// UI command: open the Stencil window (Window ▸ Stencil Settings…).
pub const STENCIL_SETTINGS: &str = "ui.stencilSettings";
/// UI command: Object ▸ Physics ▸ Settings…: give the selected objects physics settings when they
/// have none and open them in the Appearance panel.
pub const PHYSICS_SETTINGS: &str = "ui.physicsSettings";

/// Picabia's UI state (not the document).
#[derive(Default)]
pub struct PicabiaUi {
    /// A Shift+Esc press the window's raw input held, for the laser's stop (`picabia_laser_window::run`).
    pub stop_pressed: std::cell::Cell<bool>,
    /// Tests: the frames carry injected input (`picabia_laser_window::run::automated_frame`).
    #[cfg(test)]
    pub injected: bool,
    /// The laser export dialog, while it is open (`picabia_laser_export`).
    pub export: Option<crate::picabia_laser_export::Dialog>,
    /// The Laser window.
    pub laser: crate::picabia_laser_window::LaserPanel,
    /// The Camera window.
    pub camera: crate::picabia_camera_window::CameraPanel,
    /// The Stencil window is open.
    pub stencil_open: bool,
    /// The Appearance panel's physics row is open (`picabia::physics`).
    pub physics_open: bool,
    /// A poke is held on the canvas (`picabia::sim_poke::poke`).
    pub poking: bool,
    /// Where the pointer last pulled the poked body (screen), so holding still sends nothing.
    pub poke_at: Option<egui::Pos2>,
    /// The Properties panel's Modifiers section is collapsed (`picabia::properties`).
    pub modifiers_collapsed: bool,
    /// The Properties panel's Modifiers section is to open and scroll into view on its next frame
    /// (the canvas badge and the task bar's Show Modifiers ask for it).
    pub reveal_modifiers: bool,
}

/// Picabia's UI commands (seam in `menus::run_ui_command`); `None`: not one of them.
pub fn command(app: &mut VectorcraftApp, id: &str, _p: &Value) -> Option<Result<Value, String>> {
    if let Some(r) = crate::picabia_camera_window::command(app, id) {
        return Some(r);
    }
    match id {
        LASER_PANEL => {
            crate::picabia_laser_window::open(app);
            Some(Ok(serde_json::json!({"open": true})))
        }
        STENCIL_SETTINGS => {
            app.picabia.stencil_open = true;
            Some(Ok(serde_json::json!({"open": true})))
        }
        PHYSICS_SETTINGS => Some(physics_settings(app)),
        _ => None,
    }
}

/// Object ▸ Physics ▸ Settings…: settings for the selected objects that have none (one undo step;
/// those with settings keep theirs), then the Appearance panel with the physics row open.
fn physics_settings(app: &mut VectorcraftApp) -> Result<Value, String> {
    let added = app.run("physics.set", serde_json::json!({}))?;
    app.picabia.physics_open = true;
    app.run("window.panel", serde_json::json!({"panel": "appearance"}))?;
    Ok(added)
}

/// After engine command `id` returned `v` (seam in `run_now`): a laser export run without a path
/// asks for its dialog (`{dialog: "stencilExport" | "stencilTestCard" | "laserGcode", …}`).
pub fn after_command(app: &mut VectorcraftApp, id: &str, v: &Value) {
    crate::picabia_laser_export::open_requested(app, id, v);
}

/// Everything Picabia draws over the app each frame (seam in `ui_frame`, after the progress
/// windows): the stencil's canvas markers, the Stencil window,
/// the laser export dialog, the Laser window (or its bar while connected and closed) and the safety
/// checklist.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    crate::picabia_stencil_panel::canvas_markers(app, ctx);
    crate::picabia_stencil_panel::window(app, ctx);
    crate::picabia_laser_export::show(app, ctx);
    crate::picabia_laser_window::show(app, ctx);
    crate::picabia_camera_window::show(app, ctx);
    crate::picabia::sim_bar::show(app, ctx);
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

/// Lays the active document's canvas out at `rect`, as a frame of the window would.
#[cfg(test)]
pub(crate) fn set_canvas(app: &mut VectorcraftApp, rect: egui::Rect) {
    app.sync_views();
    if let Some(v) = app.view_mut() {
        v.rect = rect;
    }
}

/// A context with the app's interface installed in the dark or the light theme (Picabia's look
/// tests draw in both).
#[cfg(test)]
pub(crate) fn themed(dark: bool) -> egui::Context {
    let ctx = crate::testing::ui_context();
    let prefs = vectorcraft_engine::Prefs { ui_brightness: if dark { "dark" } else { "light" }.into(), ..Default::default() };
    craft::ui::runtime::frame::set_input(&ctx, craft::ui::runtime::frame::FrameInput::new(crate::app::root_env(&prefs, 1.0)));
    ctx
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
        // Physics Settings… needs a selection; with one it adds settings and opens the row.
        assert!(app.run(PHYSICS_SETTINGS, serde_json::json!({})).is_err());
        app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", serde_json::json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone();
        assert!(app.run(PHYSICS_SETTINGS, serde_json::json!({})).is_ok());
        assert!(app.picabia.physics_open);
        assert_eq!(app.run("physics.get", serde_json::json!({"id": id})).unwrap()["settings"]["bodyType"], "dynamic");
    }
}
