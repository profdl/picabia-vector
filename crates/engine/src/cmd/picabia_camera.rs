//! PICABIA: the camera link (`camera.*` commands): live view from a Panasonic Lumix camera (or
//! the simulated one, address `sim`) and captures saved as numbered JPEGs in a folder the user
//! chooses. See `docs/picabia/camera.md`.
//!
//! # Threads
//! Natively a worker thread owns the network and a [`vectorcraft_lumix::Controller`]; commands
//! send it messages and read a shared copy of its state, so no command waits on the camera for
//! longer than a capture takes ([`CAPTURE_TIMEOUT_MS`] at most). The worker decodes each new frame
//! once, for the Camera window. The browser build has no sockets: `camera.status` reports closed
//! and the rest are refused.
//!
//! # The network
//! Until the desktop app calls [`Session::camera_enable_network`] (or the environment sets
//! `PICABIA_CAMERA_NETWORK=1`, for the CLI and MCP), only the simulator can be connected: tests
//! and the command sweep never touch the network.
//!
//! # Captures
//! `camera.capture` takes the newest live-view frame, fires the shutter, and once the camera says
//! it fired, writes the frame as `<prefix>_<NNNN>.jpg` in the capture folder (`NNNN` one more
//! than the highest already there). The full-resolution photo stays on the camera's card until
//! PIC34.7 downloads it. Placing the file on the canvas is the Camera window's job (`file.place`
//! with `link: true`): this command never touches the document.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd};
use crate::{EngineError, Result, Session};

/// The camera's address on its own Wi-Fi network.
pub use vectorcraft_lumix::DEFAULT_IP;

/// Why the browser build refuses.
pub const DESKTOP_ONLY: &str = "the camera needs the desktop app";
/// The simulated camera's address.
pub const SIM_ADDRESS: &str = "sim";
/// Why a real camera is refused before the network is turned on.
pub const NO_NETWORK: &str = "a real camera can be reached only from the desktop app (or with PICABIA_CAMERA_NETWORK=1); this session can use the simulated camera, ip \"sim\"";
/// Why a capture is refused without a folder.
pub const NO_FOLDER: &str = "choose a capture folder first (Window ▸ Camera…, or camera.settings {folder})";
/// The environment variable that lets the CLI and MCP reach a real camera.
pub const NETWORK_ENV: &str = "PICABIA_CAMERA_NETWORK";
/// The longest a capture waits for the camera to say it fired.
pub const CAPTURE_TIMEOUT_MS: u64 = 5_000;
/// The file-name prefix when none is set.
pub const DEFAULT_PREFIX: &str = "frame";
/// The longest prefix.
pub const MAX_PREFIX: usize = 40;
/// The most directory entries looked at to find the next number.
const MAX_SCAN: usize = 100_000;
/// The highest frame number.
const MAX_NUMBER: u32 = 999_999;

/// What the camera link remembers (`Prefs::picabia_camera`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CameraPrefs {
    /// The last camera address connected to.
    pub ip: Option<String>,
    /// The capture folder (`None`: not chosen yet).
    pub folder: Option<String>,
    /// The file-name prefix (empty: [`DEFAULT_PREFIX`]).
    pub prefix: String,
    /// Our id in the camera's access handshake, made once.
    pub device_id: Option<String>,
    /// The Camera window also places each capture on the canvas, linked.
    pub place: bool,
}

impl CameraPrefs {
    pub fn read(s: &Session) -> CameraPrefs {
        serde_json::from_value(s.prefs.picabia_camera.clone()).unwrap_or_default()
    }

    fn write(&self, s: &mut Session) {
        s.prefs.picabia_camera = serde_json::to_value(self).unwrap_or(Value::Null);
    }

    pub fn prefix(&self) -> &str {
        if self.prefix.is_empty() { DEFAULT_PREFIX } else { &self.prefix }
    }
}

/// A decoded live-view frame, for the Camera window.
#[derive(Clone, Debug)]
pub struct CameraFrame {
    /// Its number (one more for each frame received).
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, `width × height × 4` bytes.
    pub rgba: Arc<Vec<u8>>,
}

/// The session's camera link. See the module docs.
#[derive(Debug, Default)]
pub struct CameraLink {
    /// Real cameras may be connected ([`Session::camera_enable_network`]).
    network: bool,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<native::Worker>,
}

impl Session {
    /// PICABIA: let this session reach real cameras. The desktop app calls it once at startup.
    pub fn camera_enable_network(&mut self) {
        self.picabia.camera.network = true;
    }

    /// PICABIA: the app is quitting: ask the camera to stop streaming and end the worker (never
    /// waits on the camera).
    pub fn camera_shutdown(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(w) = self.picabia.camera.worker.take() {
            w.close();
        }
    }

    /// PICABIA: the newest decoded live-view frame (the Camera window's picture).
    pub fn camera_frame(&self) -> Option<CameraFrame> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.picabia.camera.worker.as_ref().and_then(native::Worker::decoded)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
}

impl CameraLink {
    fn network_allowed(&self) -> bool {
        self.network || std::env::var(NETWORK_ENV).is_ok_and(|v| v.trim() == "1")
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "camera.status",
            "Camera Status",
            [],
            None,
            "{} → {connected, phase (closed|connecting|connected|streaming|lost), ip, sim, network (real cameras allowed), liveView, fps, frames (received so far), dropped (datagrams without a frame), lastDrop, noFrames (streaming but nothing arriving), hint (what to try, plain) | null, lastError, failures, retryInSec | null, handshaken, camera {batt, cammode, …: the camera's last getstate} , folder | null, prefix, nextFile | null, place, log [the last 50 requests and answers]}",
            always,
            status
        ),
        cmd!(
            query "camera.connect",
            "Connect to Camera",
            [],
            None,
            "{ip? (the camera's IP address; default the last one, else 192.168.54.1, the camera's own Wi-Fi; \"sim\": the simulated camera), sim? (ip \"sim\" only: {requireHandshake, offline, refuseCapture, fps (1-60), streamTimeoutMs})} (starts the link and live view; poll camera.status until phase is streaming; a camera that doesn't answer is tried again with a backoff until camera.disconnect; real cameras only from the desktop app or with PICABIA_CAMERA_NETWORK=1) → camera.status",
            always,
            connect
        ),
        cmd!(query "camera.disconnect", "Disconnect Camera", [], None, "{} (asks the camera to stop streaming and closes the link) → camera.status", always, disconnect),
        cmd!(query "camera.start", "Start Live View", [], None, "{} (live view on; it starts once connected) → camera.status", always, start),
        cmd!(query "camera.stop", "Stop Live View", [], None, "{} (live view off; the link stays connected and captures still work) → camera.status", always, stop),
        cmd!(
            query "camera.capture",
            "Capture Frame",
            [],
            None,
            "{} (fires the shutter; once the camera says it fired, saves the live-view frame taken just before as <prefix>_<NNNN>.jpg in the capture folder, NNNN one more than the highest there; the full-resolution photo stays on the camera's card; needs a folder, see camera.settings; never changes the document) → {shutter: true, saved, path | null, number | null, file | null, warnings}",
            always,
            capture
        ),
        cmd!(
            query "camera.settings",
            "Camera Settings",
            [],
            None,
            "{folder? (an existing, writable folder for captures), prefix? (file-name prefix: 1-40 letters, digits, spaces, - or _; empty: \"frame\"), place? (bool: the Camera window also places each capture on the canvas as a linked image)} (remembered with the preferences) → camera.status",
            always,
            settings
        ),
        cmd!(
            query "camera.frame",
            "Camera Frame",
            [],
            None,
            "{} the newest live-view frame → {seq, bytes, jpegBase64} (an error when none has arrived)",
            always,
            frame
        ),
        cmd!(
            query "camera.find",
            "Find Camera",
            [],
            None,
            "{timeoutMs? (200-10000, default 3000)} (SSDP search on this computer's networks, for a camera that joined another network; waits the whole timeout; real network only, as camera.connect) → {cameras: [{ip, likelyLumix, server, location}]} (likely cameras first)",
            always,
            find
        ),
    ]
}

// ---------- the commands ----------

fn disabled(cmd: &str, why: impl Into<String>) -> EngineError {
    EngineError::Disabled(cmd.into(), why.into())
}

/// The value `p` gives for `k`, null counting as not given.
fn get<'a>(p: &'a Value, k: &str) -> Option<&'a Value> {
    p.get(k).filter(|v| !v.is_null())
}

pub fn status_json(s: &Session) -> Value {
    let prefs = CameraPrefs::read(s);
    let next = prefs.folder.as_deref().and_then(|f| next_number(Path::new(f), prefs.prefix()).ok()).map(|n| file_name(prefs.prefix(), n));
    let mut v = link_json(s);
    if let Some(o) = v.as_object_mut() {
        o.insert("network".into(), json!(s.picabia.camera.network_allowed()));
        o.insert("folder".into(), json!(prefs.folder));
        o.insert("prefix".into(), json!(prefs.prefix()));
        o.insert("nextFile".into(), json!(next));
        o.insert("place".into(), json!(prefs.place));
    }
    v
}

/// The link's part of the status.
fn link_json(s: &Session) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = s.picabia.camera.worker.as_ref() {
        return w.status_json();
    }
    let _ = s;
    json!({
        "connected": false, "phase": "closed", "ip": null, "sim": false, "liveView": false, "fps": 0, "frames": 0,
        "dropped": 0, "lastDrop": null, "noFrames": false, "hint": null, "lastError": null, "failures": 0,
        "retryInSec": null, "handshaken": false, "camera": {}, "log": [],
    })
}

fn status(s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(status_json(s))
}

fn connect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "camera.connect";
    let mut prefs = CameraPrefs::read(s);
    let ip = match get(p, "ip") {
        None => prefs.ip.clone().unwrap_or_else(|| vectorcraft_lumix::DEFAULT_IP.into()),
        Some(Value::String(ip)) if ip.len() <= 64 => ip.trim().to_string(),
        Some(_) => return Err(bad(CMD, "`ip` must be the camera's IP address (e.g. 192.168.54.1) or \"sim\"")),
    };
    let sim = get(p, "sim");
    if sim.is_some() && ip != SIM_ADDRESS {
        return Err(bad(CMD, "`sim` options go with ip \"sim\" only"));
    }
    if sim.is_some_and(|v| !v.is_object()) {
        return Err(bad(CMD, "`sim` must be an object of simulator options"));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, prefs);
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let target = if ip == SIM_ADDRESS {
            native::Target::Sim(native::sim_options(sim).map_err(|e| bad(CMD, e))?)
        } else {
            let addr = vectorcraft_lumix::net::parse_ip(&ip).map_err(|e| bad(CMD, e))?;
            if !s.picabia.camera.network_allowed() {
                return Err(disabled(CMD, NO_NETWORK));
            }
            native::Target::Camera(addr)
        };
        let device_id = prefs.device_id.clone().unwrap_or_else(new_device_id);
        if let Some(w) = s.picabia.camera.worker.take() {
            w.close();
        }
        let w = native::Worker::start(target, &ip, &device_id).map_err(|e| disabled(CMD, e))?;
        s.picabia.camera.worker = Some(w);
        if ip != SIM_ADDRESS {
            prefs.ip = Some(ip);
        }
        prefs.device_id = Some(device_id);
        prefs.write(s);
        Ok(status_json(s))
    }
}

fn disconnect(s: &mut Session, _p: &Value) -> Result<Value> {
    s.camera_shutdown();
    Ok(status_json(s))
}

fn start(s: &mut Session, _p: &Value) -> Result<Value> {
    live_view(s, "camera.start", true)
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    live_view(s, "camera.stop", false)
}

fn live_view(s: &mut Session, cmd: &str, on: bool) -> Result<Value> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let w = s.picabia.camera.worker.as_ref().ok_or_else(|| disabled(cmd, "the camera isn't connected"))?;
        w.live_view(on).map_err(|e| disabled(cmd, e))?;
        Ok(status_json(s))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, on);
        Err(disabled(cmd, DESKTOP_ONLY))
    }
}

fn capture(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "camera.capture";
    let prefs = CameraPrefs::read(s);
    let folder = prefs.folder.clone().ok_or_else(|| disabled(CMD, NO_FOLDER))?;
    check_folder(Path::new(&folder)).map_err(|e| disabled(CMD, e))?;
    #[cfg(target_arch = "wasm32")]
    {
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let w = s.picabia.camera.worker.as_ref().ok_or_else(|| disabled(CMD, "the camera isn't connected"))?;
        let jpeg = w.capture(CAPTURE_TIMEOUT_MS).map_err(|e| disabled(CMD, e))?;
        let Some(jpeg) = jpeg else {
            return Ok(json!({
                "shutter": true, "saved": false, "path": null, "number": null, "file": null,
                "warnings": ["live view is off, so no frame was saved (the photo is on the camera's card)"],
            }));
        };
        let (path, number) = save_frame(Path::new(&folder), prefs.prefix(), &jpeg).map_err(|e| disabled(CMD, e))?;
        let file = path.file_name().map(|f| f.to_string_lossy().to_string());
        Ok(json!({"shutter": true, "saved": true, "path": path.to_string_lossy(), "number": number, "file": file, "warnings": []}))
    }
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "camera.settings";
    let mut prefs = CameraPrefs::read(s);
    if let Some(v) = get(p, "folder") {
        let f = v.as_str().filter(|f| !f.trim().is_empty() && f.len() <= 4096).ok_or_else(|| bad(CMD, "`folder` must be a folder path"))?;
        check_folder(Path::new(f)).map_err(|e| bad(CMD, e))?;
        prefs.folder = Some(f.to_string());
    }
    if let Some(v) = get(p, "prefix") {
        let raw = v.as_str().ok_or_else(|| bad(CMD, "`prefix` must be text"))?;
        prefs.prefix = check_prefix(raw).map_err(|e| bad(CMD, e))?;
    }
    if let Some(v) = get(p, "place") {
        prefs.place = v.as_bool().ok_or_else(|| bad(CMD, "`place` must be true or false"))?;
    }
    prefs.write(s);
    Ok(status_json(s))
}

fn frame(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "camera.frame";
    #[cfg(not(target_arch = "wasm32"))]
    {
        let w = s.picabia.camera.worker.as_ref().ok_or_else(|| disabled(CMD, "the camera isn't connected"))?;
        let (seq, jpeg) = w.jpeg().ok_or_else(|| disabled(CMD, "no frame has arrived yet"))?;
        Ok(json!({"seq": seq, "bytes": jpeg.len(), "jpegBase64": vectorcraft_format::base64_encode(&jpeg)}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        Err(disabled(CMD, DESKTOP_ONLY))
    }
}

fn find(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "camera.find";
    let timeout = match get(p, "timeoutMs") {
        None => 3000,
        Some(v) => {
            v.as_f64().filter(|t| t.is_finite() && (200.0..=10_000.0).contains(t)).ok_or_else(|| bad(CMD, "`timeoutMs` must be 200-10000"))? as u64
        }
    };
    if !s.picabia.camera.network_allowed() {
        return Err(disabled(CMD, NO_NETWORK));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let found = vectorcraft_lumix::net::discover(timeout).map_err(|e| disabled(CMD, e))?;
        let cams: Vec<Value> =
            found.iter().map(|f| json!({"ip": f.ip, "likelyLumix": f.likely_lumix, "server": f.server, "location": f.location})).collect();
        Ok(json!({"cameras": cams}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = timeout;
        Err(disabled(CMD, DESKTOP_ONLY))
    }
}

// ---------- folders and numbers ----------

/// `f` is an existing folder we may write to.
pub fn check_folder(f: &Path) -> std::result::Result<(), String> {
    let meta = std::fs::metadata(f).map_err(|_| format!("the folder {} doesn't exist", f.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} isn't a folder", f.display()));
    }
    if meta.permissions().readonly() {
        return Err(format!("the folder {} is read-only", f.display()));
    }
    Ok(())
}

/// A prefix made safe for file names: trimmed, 1-40 letters, digits, spaces, `-` or `_` (empty:
/// the default).
pub fn check_prefix(raw: &str) -> std::result::Result<String, String> {
    let p = raw.trim();
    if p.chars().count() > MAX_PREFIX {
        return Err(format!("the prefix can be at most {MAX_PREFIX} characters"));
    }
    if !p.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
        return Err("the prefix may hold only letters, digits, spaces, - and _".into());
    }
    Ok(p.to_string())
}

/// `<prefix>_<NNNN>.jpg`.
pub fn file_name(prefix: &str, n: u32) -> String {
    format!("{prefix}_{n:04}.jpg")
}

/// One more than the highest `<prefix>_<digits>.jpg` (or `.jpeg`, any case) in `folder`; 1 when
/// there is none.
pub fn next_number(folder: &Path, prefix: &str) -> std::result::Result<u32, String> {
    let rd = std::fs::read_dir(folder).map_err(|e| format!("can't read {}: {e}", folder.display()))?;
    let lead = format!("{prefix}_");
    let mut max = 0u32;
    for entry in rd.take(MAX_SCAN).flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(n) = number_in(&name, &lead) {
            max = max.max(n);
        }
    }
    if max >= MAX_NUMBER {
        return Err(format!("the folder already has frame {MAX_NUMBER}: choose another folder or prefix"));
    }
    Ok(max + 1)
}

fn number_in(name: &str, lead: &str) -> Option<u32> {
    let rest = name.strip_prefix(lead)?;
    let (digits, ext) = rest.rsplit_once('.')?;
    if !matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg")
        || digits.is_empty()
        || digits.len() > 6
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// Write `jpeg` as the next numbered frame in `folder`, without ever replacing a file: a name
/// taken in the meantime moves on to the next number.
pub fn save_frame(folder: &Path, prefix: &str, jpeg: &[u8]) -> std::result::Result<(PathBuf, u32), String> {
    use std::io::Write as _;
    let mut n = next_number(folder, prefix)?;
    for _ in 0..100 {
        let path = folder.join(file_name(prefix, n));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(jpeg).and_then(|()| f.sync_all()).map_err(|e| format!("can't write {}: {e}", path.display()))?;
                return Ok((path, n));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(format!("can't write {}: {e}", path.display())),
        }
    }
    Err("couldn't find a free file name".into())
}

/// A new id for the camera's access handshake: the UUID shape the camera expects, from the clock
/// and a per-process random hash (not secret: it only names this computer to the camera).
fn new_device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default());
    h.write_u32(std::process::id());
    format!("4D454930-0100-1000-8000-{:012X}", h.finish() & 0xFFFF_FFFF_FFFF)
}
