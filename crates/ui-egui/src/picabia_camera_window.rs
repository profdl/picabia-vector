//! PICABIA: the Camera window: connect to a Panasonic Lumix camera (or the simulated one, address
//! `sim`), watch its live view, and capture numbered frames into a folder the user chooses,
//! optionally placing each one on the canvas as a linked image. A floating window opened by
//! `camera.panel` (Window ▸ Camera…). Every action is a `camera.*` command or one of the UI
//! commands here ([`capture`], [`choose_folder`]), so a shortcut, the palette, the control channel
//! and MCP do what the buttons do. Live view is shown here only: nothing is drawn on the canvas.

use egui::{Context, Id, TextureHandle, TextureOptions, Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::picabia_ui::{button, f, note, readable, s, wrapped};
use crate::theme::Tokens;

/// UI command: open the Camera window.
pub const CAMERA_PANEL: &str = "camera.panel";
/// UI command: capture to the folder (the folder picker first when none is chosen), then place
/// the frame on the canvas when the window's option is on.
pub const CAMERA_CAPTURE: &str = "ui.cameraCapture";
/// UI command: choose the capture folder.
pub const CAMERA_FOLDER: &str = "ui.cameraFolder";
/// The live view's width in the window.
const VIEW_W: f32 = 480.0;
/// How often the window redraws while connected (ms): live view moves.
const REPAINT_MS: u64 = 33;

/// The Camera window's state (UI state, not the document).
#[derive(Default)]
pub struct CameraPanel {
    pub open: bool,
    /// The address field (filled from the preferences when the window opens).
    pub ip: String,
    /// The prefix field.
    pub prefix: String,
    /// The fields were filled from the preferences.
    loaded: bool,
    texture: Option<TextureHandle>,
    texture_seq: u64,
    /// The last capture or error, for the window.
    pub last: Option<(bool, String)>,
    /// What `camera.find` found: (ip, label).
    pub found: Vec<(String, String)>,
}

impl std::fmt::Debug for CameraPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraPanel").field("open", &self.open).field("ip", &self.ip).finish()
    }
}

/// Picabia's camera UI commands; `None`: not one of them.
pub fn command(app: &mut VectorcraftApp, id: &str) -> Option<Result<Value, String>> {
    match id {
        CAMERA_PANEL => {
            open(app);
            Some(Ok(json!({"open": true})))
        }
        CAMERA_CAPTURE => Some(capture(app)),
        CAMERA_FOLDER => Some(choose_folder(app).map(|f| json!({"folder": f}))),
        _ => None,
    }
}

pub fn open(app: &mut VectorcraftApp) {
    let p = &mut app.picabia.camera;
    p.open = true;
    p.loaded = false;
}

/// Choose the capture folder with the folder picker; the folder (`None`: cancelled, or the
/// picker shows off the UI thread and runs this again with the answer).
pub fn choose_folder(app: &mut VectorcraftApp) -> Result<Option<String>, String> {
    let Some(folder) = crate::picks::folder(app) else { return Ok(None) };
    app.run("camera.settings", json!({"folder": folder}))?;
    app.prefs_changed = true;
    Ok(Some(folder))
}

/// Capture (see [`CAMERA_CAPTURE`]).
pub fn capture(app: &mut VectorcraftApp) -> Result<Value, String> {
    let status = app.run("camera.status", json!({}))?;
    if status["folder"].is_null() && choose_folder(app)?.is_none() {
        return Ok(json!({"saved": false, "cancelled": true}));
    }
    let r = app.run("camera.capture", json!({}));
    let shown = match &r {
        Ok(v) if v["saved"] == true => (true, format!("{} {}", tl!("Saved"), s(v, "file").unwrap_or_default())),
        Ok(v) => (false, crate::picabia_ui::strings(v, "warnings", 1).join(" ")),
        Err(e) => (false, e.clone()),
    };
    app.picabia.camera.last = Some(shown);
    let v = r?;
    if v["saved"] == true && status["place"] == true {
        place(app, &v)?;
    }
    Ok(v)
}

/// Place a saved capture on the active artboard, fitted inside it, linked to its file.
fn place(app: &mut VectorcraftApp, captured: &Value) -> Result<(), String> {
    let Some(path) = s(captured, "path") else { return Ok(()) };
    let Some(doc) = app.session.active().map(|d| &d.doc) else {
        app.status(tl!("The frame was saved; open a document to place captures on the canvas"));
        return Ok(());
    };
    let n = doc.artboards.len();
    let rect = (n > 0).then(|| crate::panels::artboards::selected(app, n)).and_then(|i| app.session.active()?.doc.artboards.get(i).map(|a| a.rect));
    let mut p = json!({"path": path, "link": true});
    if let (Some(r), Some(o)) = (rect, p.as_object_mut()) {
        o.insert("rect".into(), json!([r.x0, r.y0, r.width(), r.height()]));
    }
    app.run("file.place", p).map(|_| ())
}

/// What the window does after drawing (the window can't run commands while it borrows the app).
enum Action {
    Connect,
    Disconnect,
    Find,
    LiveView(bool),
    Capture,
    Folder,
    Prefix,
    Place(bool),
    UseIp(String),
}

/// The window (from `picabia_ui::show`).
pub fn show(app: &mut VectorcraftApp, ctx: &Context) {
    if !app.picabia.camera.open {
        return;
    }
    let st = vectorcraft_engine::cmd::picabia_camera::status_json(&app.session);
    if st["connected"] == true {
        ctx.request_repaint_after(std::time::Duration::from_millis(REPAINT_MS));
    }
    let mut panel = std::mem::take(&mut app.picabia.camera);
    if !panel.loaded {
        panel.ip = s(&st, "ip")
            .filter(|ip| ip != "sim")
            .or_else(|| last_ip(app))
            .unwrap_or_else(|| vectorcraft_engine::cmd::picabia_camera::DEFAULT_IP.into());
        panel.prefix = s(&st, "prefix").unwrap_or_default();
        panel.loaded = true;
    }
    update_texture(app, ctx, &mut panel);
    let mut actions = Vec::new();
    let mut open = true;
    let t = Tokens::get(ctx);
    let room = ctx.content_rect();
    egui::Window::new(tl!("Camera"))
        .id(Id::unique("picabia_camera_window"))
        .collapsible(false)
        .resizable(false)
        .default_pos(room.right_top() + vec2(-VIEW_W - 120.0, 96.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(14)))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(VIEW_W);
            if cfg!(target_arch = "wasm32") {
                wrapped(ui, tl!("The camera needs the desktop app."), t.text_dim);
                return;
            }
            body(ui, &mut panel, &st, &t, &mut actions);
        });
    let asked = std::mem::take(&mut app.picabia.camera).open;
    panel.open = open || asked;
    app.picabia.camera = panel;
    for a in actions {
        act(app, a);
    }
}

fn last_ip(app: &VectorcraftApp) -> Option<String> {
    vectorcraft_engine::cmd::picabia_camera::CameraPrefs::read(&app.session).ip
}

/// Upload a new live-view frame to the window's texture.
fn update_texture(app: &VectorcraftApp, ctx: &Context, panel: &mut CameraPanel) {
    let Some(frame) = app.session.camera_frame() else { return };
    if panel.texture.is_some() && frame.seq == panel.texture_seq {
        return;
    }
    let (Ok(w), Ok(h)) = (usize::try_from(frame.width), usize::try_from(frame.height)) else { return };
    if frame.rgba.len() != w * h * 4 {
        return;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &frame.rgba);
    match panel.texture.as_mut() {
        Some(tex) => tex.set(image, TextureOptions::LINEAR),
        None => panel.texture = Some(ctx.load_texture("picabia_camera_live", image, TextureOptions::LINEAR)),
    }
    panel.texture_seq = frame.seq;
}

fn body(ui: &mut Ui, panel: &mut CameraPanel, st: &Value, t: &Tokens, actions: &mut Vec<Action>) {
    let connected = st["connected"] == true;
    live_view(ui, panel, st, t);
    ui.add_space(6.0);
    status_line(ui, st, t);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Camera"));
        if connected {
            // The address actually connected (the simulator's too), not the field's.
            let mut ip = s(st, "ip").unwrap_or_default();
            ui.add_enabled(false, egui::TextEdit::singleline(&mut ip).desired_width(150.0));
        } else {
            ui.add(egui::TextEdit::singleline(&mut panel.ip).desired_width(150.0).hint_text(vectorcraft_engine::cmd::picabia_camera::DEFAULT_IP));
        }
        if connected {
            if button(ui, tl!("Disconnect"), false, true) {
                actions.push(Action::Disconnect);
            }
        } else if button(ui, tl!("Connect"), true, !panel.ip.trim().is_empty()) {
            actions.push(Action::Connect);
        }
        if button(ui, tl!("Find"), false, !connected) {
            actions.push(Action::Find);
        }
    });
    if !connected && !panel.found.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (ip, label) in &panel.found {
                if ui.small_button(label).clicked() {
                    actions.push(Action::UseIp(ip.clone()));
                }
            }
        });
    }
    note(ui, tl!("On the camera's own Wi-Fi it is 192.168.54.1. \"sim\" is a simulated camera."));
    ui.add_space(10.0);
    capture_rows(ui, panel, st, t, actions);
}

fn live_view(ui: &mut Ui, panel: &CameraPanel, st: &Value, t: &Tokens) {
    let size = vec2(VIEW_W, VIEW_W * 0.75);
    match (&panel.texture, st["connected"] == true) {
        (Some(tex), true) => {
            let [w, h] = tex.size();
            let aspect = if w > 0 { h as f32 / w as f32 } else { 0.75 };
            ui.add(egui::Image::new(tex).fit_to_exact_size(vec2(VIEW_W, VIEW_W * aspect)));
        }
        _ => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(rect, 4.0, t.panel_darker);
            let text = if st["connected"] == true { tl!("Waiting for live view…") } else { tl!("No camera connected") };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), t.text_dim);
        }
    }
}

fn status_line(ui: &mut Ui, st: &Value, t: &Tokens) {
    let phase = s(st, "phase").unwrap_or_default();
    let mut line = if st["awaitingAccept"] == true { tl!("Waiting for the camera to accept…") } else { phase_label(&phase) }.to_string();
    if phase == "streaming" {
        line.push_str(&format!(" · {} fps · {} {}", f(st, "fps").unwrap_or(0.0), f(st, "frames").unwrap_or(0.0), tl!("frames")));
    }
    if let Some(b) = st["camera"]["batt"].as_str() {
        line.push_str(&format!(" · {} {b}", tl!("battery")));
    }
    if let Some(secs) = f(st, "retryInSec") {
        line.push_str(&format!(" · {} {secs} s", tl!("trying again in")));
    }
    note(ui, &line);
    if let Some(h) = s(st, "hint") {
        wrapped(ui, &h, readable(t.warning, t));
    } else if let Some(e) = s(st, "lastError").filter(|_| phase != "closed") {
        wrapped(ui, &e, t.text_dim);
    }
}

fn phase_label(phase: &str) -> &'static str {
    match phase {
        "connecting" => tl!("Connecting…"),
        "connected" => tl!("Connected, live view off"),
        "streaming" => tl!("Live view"),
        "lost" => tl!("No answer"),
        _ => tl!("Not connected"),
    }
}

fn capture_rows(ui: &mut Ui, panel: &mut CameraPanel, st: &Value, t: &Tokens, actions: &mut Vec<Action>) {
    let connected = st["connected"] == true;
    let ready = matches!(st["phase"].as_str(), Some("connected" | "streaming"));
    ui.horizontal(|ui| {
        ui.label(tl!("Folder"));
        let folder = s(st, "folder").unwrap_or_else(|| tl!("Not chosen").to_string());
        ui.add(egui::Label::new(egui::RichText::new(folder).size(11.5)).truncate()).on_hover_text(s(st, "folder").unwrap_or_default());
    });
    ui.horizontal(|ui| {
        if button(ui, tl!("Choose…"), false, true) {
            actions.push(Action::Folder);
        }
        ui.label(tl!("Prefix"));
        let r = ui.add(egui::TextEdit::singleline(&mut panel.prefix).desired_width(110.0).hint_text("frame"));
        if r.lost_focus() {
            actions.push(Action::Prefix);
        }
        if let Some(next) = s(st, "nextFile") {
            note(ui, &format!("{} {next}", tl!("Next:")));
        }
    });
    let mut place = st["place"] == true;
    if ui.checkbox(&mut place, tl!("Place on canvas as linked image")).changed() {
        actions.push(Action::Place(place));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if button(ui, tl!("Capture"), true, ready) {
            actions.push(Action::Capture);
        }
        let live = st["liveView"] == true;
        if live && button(ui, tl!("Stop Live View"), false, connected) {
            actions.push(Action::LiveView(false));
        }
        if !live && button(ui, tl!("Start Live View"), false, connected) {
            actions.push(Action::LiveView(true));
        }
    });
    if let Some((ok, msg)) = &panel.last {
        wrapped(ui, msg, if *ok { t.text_dim } else { readable(t.warning, t) });
    }
}

fn act(app: &mut VectorcraftApp, a: Action) {
    let r = match a {
        Action::Connect => {
            let ip = app.picabia.camera.ip.trim().to_string();
            let r = app.run("camera.connect", json!({"ip": ip}));
            app.prefs_changed = true;
            r.map(|_| ())
        }
        Action::Disconnect => app.run("camera.disconnect", json!({})).map(|_| ()),
        Action::Find => find(app),
        Action::LiveView(on) => app.run(if on { "camera.start" } else { "camera.stop" }, json!({})).map(|_| ()),
        Action::Capture => app.run(CAMERA_CAPTURE, json!({})).map(|_| ()),
        Action::Folder => app.run(CAMERA_FOLDER, json!({})).map(|_| ()),
        Action::Prefix => {
            let prefix = app.picabia.camera.prefix.clone();
            app.prefs_changed = true;
            app.run("camera.settings", json!({"prefix": prefix})).map(|_| ())
        }
        Action::Place(on) => {
            app.prefs_changed = true;
            app.run("camera.settings", json!({"place": on})).map(|_| ())
        }
        Action::UseIp(ip) => {
            app.picabia.camera.ip = ip;
            Ok(())
        }
    };
    if let Err(e) = r {
        app.picabia.camera.last = Some((false, e.clone()));
        app.status(e);
    }
}

/// Look for cameras on this computer's networks (a short search: the window waits for it).
fn find(app: &mut VectorcraftApp) -> Result<(), String> {
    let v = app.run("camera.find", json!({"timeoutMs": 1500}))?;
    let found: Vec<(String, String)> = v["cameras"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let ip = c["ip"].as_str()?.to_string();
            let label = if c["likelyLumix"] == true { format!("{ip} (Lumix)") } else { ip.clone() };
            Some((ip, label))
        })
        .collect();
    app.picabia.camera.last =
        Some((true, if found.is_empty() { tl!("No camera answered").to_string() } else { tl!("Pick a camera below").to_string() }));
    app.picabia.camera.found = found;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default())
    }

    fn wait_streaming(app: &mut VectorcraftApp) {
        for _ in 0..250 {
            let st = app.run("camera.status", json!({})).unwrap();
            if st["phase"] == "streaming" && st["frames"].as_u64().unwrap_or(0) > 1 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the simulated camera didn't stream");
    }

    #[test]
    fn opens_from_its_command() {
        let mut app = app();
        assert!(app.run(CAMERA_PANEL, json!({})).is_ok());
        assert!(app.picabia.camera.open);
    }

    #[test]
    fn draws_with_and_without_a_camera() {
        let mut app = app();
        app.picabia.camera.open = true;
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(app.picabia.camera.ip, vectorcraft_engine::cmd::picabia_camera::DEFAULT_IP, "the default address");
        app.run("camera.connect", json!({"ip": "sim"})).unwrap();
        wait_streaming(&mut app);
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.picabia.camera.texture.is_some(), "live view uploaded");
        assert!(app.picabia.camera.open);
    }

    #[test]
    fn capture_saves_and_places_linked() {
        let mut app = app();
        let dir = std::env::temp_dir().join(format!("picabia-camera-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("camera.settings", json!({"folder": dir.to_string_lossy(), "place": true})).unwrap();
        app.run("camera.connect", json!({"ip": "sim"})).unwrap();
        wait_streaming(&mut app);
        let v = app.run(CAMERA_CAPTURE, json!({})).unwrap();
        assert_eq!(v["file"], "frame_0001.jpg", "{v}");
        assert!(dir.join("frame_0001.jpg").exists());
        let info = app.run("image.info", json!({})).unwrap();
        assert_eq!(info["linked"], true, "{info}");
        assert_eq!((info["pixelWidth"].as_u64(), info["pixelHeight"].as_u64()), (Some(640), Some(480)));
        assert!(info["width"].as_f64().unwrap() <= 400.0 + 1e-6, "fitted inside the artboard: {info}");
        assert_eq!(app.picabia.camera.last.as_ref().map(|l| l.0), Some(true));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn capture_without_a_folder_asks_for_one() {
        let mut app = app();
        app.run("camera.connect", json!({"ip": "sim"})).unwrap();
        wait_streaming(&mut app);
        // The test host shows no folder picker: the capture is cancelled, nothing is fired.
        let v = app.run(CAMERA_CAPTURE, json!({})).unwrap();
        assert_eq!(v["cancelled"], true);
    }
}
