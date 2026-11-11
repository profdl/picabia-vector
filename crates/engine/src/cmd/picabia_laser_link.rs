//! PICABIA: the laser link (`laser.*` commands): connect to a GRBL laser over a serial port (or
//! the simulated GRBL machine), read its state, home, jog, frame, and stream a job with pause,
//! resume and stop.
//!
//! # Threads
//! Natively a worker thread owns the port and a [`vectorcraft_grbl::Controller`]; commands send it
//! messages and read a shared copy of its state, so no command waits on the machine for more than
//! about a second. The worker pumps as soon as bytes arrive (a serial read waits at most 20 ms),
//! never on a fixed sleep. In the browser build there is no serial port: `laser.status` reports
//! disconnected, `laser.ports` lists nothing and the rest are refused.
//!
//! # Safety
//! - **Only a person in the desktop app can turn the beam on.** `laser.start`, a beam
//!   `laser.frame` and `laser.resume` of a paused job are refused unless the session is armed, and
//!   the only way to arm it is [`Session::laser_execute_armed`]: a plain method, not a command,
//!   which arms the session for exactly the one command it runs and disarms it afterwards, whatever
//!   happens. Everything else (the control channel, MCP, the CLI, `command.batch`, actions,
//!   plug-ins) reaches the engine through [`Session::execute`], which can't arm: no command, param
//!   or channel can. The desktop UI calls it after the person has gone through the safety
//!   checklist (a test checks that no other code calls it).
//! - **Real ports only in the desktop app.** Until the desktop app calls
//!   [`Session::laser_enable_hardware`], `laser.ports` lists only the simulator and
//!   `laser.connect` refuses every real port, so tests, the CLI and headless MCP sessions never
//!   open a device. In the engine's own tests real ports can't be opened at all.
//! - A job starts only when the machine is ready and Idle, `$32 = 1`, and the job stays on the
//!   bed (the profile's, or less where `$130`/`$131` say so).
//! - `laser.stop` and `laser.pause` are always allowed. A running job is stopped (hold, reset,
//!   `M5`) on `laser.disconnect`, on a panicking command ([`Session::laser_emergency_stop`] from
//!   `run_guarded`), on a panicking UI frame (the same), on quit ([`Session::laser_shutdown`]) and
//!   when the link is dropped; a lost port fails the job at the line it reached.
// The browser build has no laser link: the job builders and status helpers go unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

mod job;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::LaserStopHandle;
#[cfg(test)]
mod knife_tests;
#[cfg(test)]
mod tests;

use kurbo::Rect;
use serde_json::{Value, json};

use job::{LaserRun, bed_mm, build_run, rect_json};
pub use job::{Machine, check_gcode, check_gcode_for, check_line, check_line_for, gcode_bounds};

use super::{CommandSpec, always, bad, cmd};
use crate::{EngineError, Result, Session};

/// Why the browser build refuses.
pub const DESKTOP_ONLY: &str = "laser control needs the desktop app";
/// A job may reach this far past the bed's edges (mm).
pub const BED_TOLERANCE_MM: f64 = 0.01;
/// Log lines `laser.status` returns.
pub const STATUS_LOG_LINES: usize = 50;
/// The simulator's port path.
pub const SIM_PORT: &str = "sim";
/// The commands that turn the beam on, the ones [`Session::laser_execute_armed`] runs.
/// PICABIA (PIC35): why a knife job or frame waits for Set origin.
pub const KNIFE_Z_UNSET: &str =
    "the knife's Z isn't zeroed: lower the blade until it just touches the sheet at its bottom-left corner, then press Set origin";

pub const BEAM_COMMANDS: [&str; 3] = ["laser.start", "laser.frame", "laser.resume"];
/// Why a beam command is refused without the arm.
pub const NOT_ARMED: &str = "the laser isn't armed: in the desktop app's Laser window, go through the safety checklist and press Start (scripts, agents, MCP and the control channel can't arm)";
/// The commands that move a real machine (or change where it thinks it is) without the beam, the
/// ones [`Session::laser_execute_person`] runs: from anywhere else they are refused on a real
/// machine (the simulator takes them from anywhere). Connecting, status, exports, `laser.jogCancel`,
/// pause and stop stay open to every channel.
pub const PERSON_COMMANDS: [&str; 5] = ["laser.home", "laser.jog", "laser.unlock", "laser.setOrigin", "laser.frame"];
/// Why a motion command is refused on a real machine from a non-UI channel.
pub const NOT_A_PERSON: &str = "on a real laser this is for the person at the desktop app's Laser window (scripts, agents, MCP and the control channel may connect, read the status, export, pause and stop)";
/// Why a real port is refused before the desktop app enables them.
pub const NO_HARDWARE: &str = "real laser ports are available only in the desktop app (this session can use the simulator, port \"sim\")";
/// How long `laser.disconnect` waits for the stop sequence before it resets outright (ms).
const DISCONNECT_BUDGET_MS: u64 = 900;
/// How long quitting (and dropping the link) waits (ms).
const SHUTDOWN_BUDGET_MS: u64 = 450;
/// The most a jog may move or reach (mm).
const MAX_JOG_MM: f64 = 5000.0;

/// What `laser.status` tells about the running job besides the controller's progress.
#[derive(Clone, Debug, Default)]
struct JobInfo {
    source: &'static str,
    sheet: Option<String>,
    layer: Option<vectorcraft_doc::NodeId>,
    estimated_sec: Option<f64>,
    bounds: Option<Rect>,
    warnings: Vec<String>,
}

/// A serial port as `laser.ports` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRow {
    pub path: String,
    pub name: String,
    pub usb: bool,
}

/// What lists the computer's serial ports (the real lister, or a test's).
pub type PortLister = fn() -> Vec<PortRow>;

/// The session's laser link. See the module docs.
pub struct LaserLink {
    /// Armed for the command [`Session::laser_execute_armed`] is running.
    armed: bool,
    /// A person in the desktop app's Laser window asked for the command running
    /// ([`Session::laser_execute_person`], [`Session::laser_execute_armed`]).
    person: bool,
    /// Tests: the open simulator counts as a real machine.
    #[cfg(test)]
    pub(crate) pretend_real: bool,
    /// Real ports may be listed and opened ([`Session::laser_enable_hardware`]).
    hardware: bool,
    lister: PortLister,
    job: Option<JobInfo>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<native::Worker>,
}

impl Default for LaserLink {
    fn default() -> Self {
        LaserLink {
            armed: false,
            person: false,
            #[cfg(test)]
            pretend_real: false,
            hardware: false,
            lister: real_ports,
            job: None,
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
        }
    }
}

impl std::fmt::Debug for LaserLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaserLink").field("armed", &self.armed).field("hardware", &self.hardware).field("open", &self.is_open()).finish()
    }
}

/// The computer's serial ports.
fn real_ports() -> Vec<PortRow> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        vectorcraft_grbl::serial::list_ports().into_iter().map(|p| PortRow { path: p.path, name: p.name, usb: p.usb }).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
}

impl Session {
    /// PICABIA: run one beam command (`laser.start`, `laser.frame`, `laser.resume`) armed. Only the
    /// desktop UI calls this, after the person has gone through the safety checklist; it is
    /// deliberately not a command, so the control channel, MCP, the CLI and scripts can't reach it.
    /// The arm lasts for this call only: it is gone when it returns, whatever happened.
    pub fn laser_execute_armed(&mut self, id: &str, params: &Value) -> Result<Value> {
        if !BEAM_COMMANDS.contains(&id) {
            return Err(EngineError::Disabled(id.into(), "only laser.start, laser.frame and laser.resume are run armed".into()));
        }
        self.picabia.laser.armed = true;
        self.picabia.laser.person = true;
        let r = self.execute(id, params);
        self.picabia.laser.armed = false;
        self.picabia.laser.person = false;
        r
    }

    /// PICABIA: run one motion command ([`PERSON_COMMANDS`]: home, jog, unlock, set origin, a
    /// beam-off frame) for the person at the desktop app's Laser window. Like
    /// [`Session::laser_execute_armed`] it is not a command, so no other channel can reach it; on a
    /// real machine those commands are refused from anywhere else.
    pub fn laser_execute_person(&mut self, id: &str, params: &Value) -> Result<Value> {
        if !PERSON_COMMANDS.contains(&id) {
            return Err(EngineError::Disabled(
                id.into(),
                "only laser.home, laser.jog, laser.unlock, laser.setOrigin and laser.frame are run for a person".into(),
            ));
        }
        self.picabia.laser.person = true;
        let r = self.execute(id, params);
        self.picabia.laser.person = false;
        r
    }

    /// PICABIA: let this session list and open real serial ports. The desktop app calls it once
    /// at startup; tests, the CLI and headless MCP sessions never do, so they reach the simulator
    /// only.
    pub fn laser_enable_hardware(&mut self) {
        self.picabia.laser.hardware = true;
    }

    /// PICABIA: a handle that stops the open connection's job or motion from any thread, without
    /// the session (the UI's always-visible Stop, a shortcut, a watchdog). `None`: not connected.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn laser_stop_handle(&self) -> Option<LaserStopHandle> {
        self.picabia.laser.stop_handle()
    }

    /// PICABIA: something went wrong (a command panicked, the UI caught a panic): disarm and stop
    /// a running job (hold, reset, `M5`). Never blocks.
    pub fn laser_emergency_stop(&mut self) {
        self.picabia.laser.armed = false;
        self.picabia.laser.stop();
    }

    /// PICABIA: the app is quitting: disarm, stop a running job, give the stop sequence at most
    /// about 500 ms to go out, close the port and end the worker.
    pub fn laser_shutdown(&mut self) {
        self.picabia.laser.armed = false;
        self.picabia.laser.close(SHUTDOWN_BUDGET_MS);
        sync_machine(self);
    }
}

/// `cmd` moves the machine: refused on a real machine unless a person asked for it in the Laser
/// window. The simulator, and a closed link (the command's own check says so), pass.
fn person_or_sim(cmd: &str, s: &Session) -> Result<()> {
    let link = &s.picabia.laser;
    if link.on_real_machine() && !link.person {
        return Err(disabled(cmd, NOT_A_PERSON));
    }
    Ok(())
}

/// Copy what the connected machine reports into `PicabiaState::laser_machine` (cleared once the
/// link is closed or lost), for the G-code commands' defaults.
fn sync_machine(s: &mut Session) {
    s.picabia.laser_machine = s.picabia.laser.reports().unwrap_or_default();
}

impl LaserLink {
    /// Use up the arm: whether the command runs armed.
    fn take_arm(&mut self) -> bool {
        std::mem::take(&mut self.armed)
    }

    /// What the open link's machine reports (`$30`, `$130`, `$131`); `None` when the link is closed.
    pub(crate) fn reports(&self) -> Option<crate::cmd::picabia_modifier::LaserMachine> {
        if !self.is_open() {
            return None;
        }
        let (travel_x_mm, travel_y_mm) = self.machine_travel();
        Some(crate::cmd::picabia_modifier::LaserMachine { s_max: self.machine_s_max(), travel_x_mm, travel_y_mm })
    }

    /// Replace the port lister (tests: never the computer's ports).
    #[cfg(test)]
    pub(crate) fn set_port_lister(&mut self, lister: PortLister) {
        self.lister = lister;
    }
}

#[cfg(target_arch = "wasm32")]
impl LaserLink {
    pub fn is_open(&self) -> bool {
        false
    }
    fn on_real_machine(&self) -> bool {
        false
    }
    fn stop(&mut self) {}
    fn close(&mut self, _budget_ms: u64) {}
    pub(crate) fn machine_s_max(&self) -> Option<f64> {
        None
    }
    pub(crate) fn machine_travel(&self) -> (Option<f64>, Option<f64>) {
        (None, None)
    }
}

/// The value `p` gives for `k`, null counting as not given.
fn get<'a>(p: &'a Value, k: &str) -> Option<&'a Value> {
    p.get(k).filter(|v| !v.is_null())
}

/// The finite number `p` gives for `k`.
fn num(p: &Value, k: &str) -> Option<f64> {
    get(p, k).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// A plain `Disabled` for `cmd`.
fn disabled(cmd: &str, why: impl Into<String>) -> EngineError {
    EngineError::Disabled(cmd.into(), why.into())
}

/// A number for G-code: 3 decimals, no exponent, no `-0`.
fn gnum(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" || s.is_empty() { "0".into() } else { s }
}

/// Whether `bounds` lies on a `bed` (w, h) bed, within [`BED_TOLERANCE_MM`]; the reason when not.
pub fn check_bed(bounds: Rect, bed: (f64, f64)) -> std::result::Result<(), String> {
    let t = BED_TOLERANCE_MM;
    let finite = [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite());
    if finite && bounds.x0 >= -t && bounds.y0 >= -t && bounds.x1 <= bed.0 + t && bounds.y1 <= bed.1 + t {
        return Ok(());
    }
    Err(format!(
        "the job reaches X {} to {}, Y {} to {} mm, off the bed (0 to {} × 0 to {} mm): move its origin or use a smaller sheet",
        gnum(bounds.x0),
        gnum(bounds.x1),
        gnum(bounds.y0),
        gnum(bounds.y1),
        gnum(bed.0),
        gnum(bed.1)
    ))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "laser.ports",
            "List Laser Ports",
            [],
            None,
            "{} → {ports: [{path (what laser.connect takes), name (USB product and maker when known), usb, sim?}]} (USB first; on macOS only /dev/cu.*; real ports only in the desktop app; the simulator {path: \"sim\", name: \"Simulated GRBL\", sim: true} in debug builds or with PICABIA_LASER_SIM=1; empty in the browser)",
            always,
            ports
        ),
        cmd!(
            query "laser.connect",
            "Connect to Laser",
            [],
            None,
            "{port (a path from laser.ports, or \"sim\"), baud? (default 115200), sim? (testing aid, port \"sim\" only: {laserMode (bool, $32), sMax ($30), maxTravelMm [x, y, z] ($130-$132), speedFactor (moves this many times faster, 0.01-1e6), timeScale (the simulated clock runs this many times faster, 1-1000), alarmAtLine, errorAtLine (raise ALARM:1 / answer error:20 at that G-code line), disconnectAtLine (the port fails once the machine has read that many G-code lines), homing ($22), startInAlarm, bannerOnOpen, rapidMmMin}; default the owner's X-Carve: laser mode on, $30 = 1000)} (opens the port and starts the link; it then waits for the banner and reads $I and $$: poll laser.status until phase is ready; refused while a job runs; real ports only in the desktop app) → laser.status",
            always,
            connect
        ),
        cmd!(
            query "laser.disconnect",
            "Disconnect Laser",
            [],
            None,
            "{} (a running job is stopped first: hold, reset, M5; the port is closed) → laser.status",
            always,
            disconnect
        ),
        cmd!(
            query "laser.status",
            "Laser Status",
            [],
            None,
            "{} → {connected, phase (closed|connecting|ready|lost), port, portName, sim, hardware (real ports allowed), state (Idle|Run|Hold|Jog|Alarm|Door|Check|Home|Sleep|Unknown, null before the first report), pos {x, y, z} (work, mm) | null, mpos {x, y, z} | null, feed, power (S) | null, armed, stopping, job {name, state (running|paused|done|stopped|failed), line (lines answered), lines, sent, percent, elapsedSec, etaSec | null, estimatedSec | null, why?, failedLine?, source (gcode|file|testCard|powerLadder|frame), boundsMm {x0, y0, x1, y1} | null, warnings} | null, alarm {code, text, advice} | null, settings {sMax ($30) | null, laserMode ($32) | null, maxTravel [x, y, z] | null, homing | null, softLimits | null, version | null, build | null, rxSize}, profile {name, bedWidthMm, bedHeightMm, sMax, rapidMmMin, air, checked, origin (head|machine), head (laser|knife), knife {safeZMm, cutZMm, score, scoreZMm, feedMmMin, plungeMmMin, passes, offsetMm, swivelDeg, overcutMm}}, knifeZSet (Set origin zeroed Z with the knife on this connection; knife jobs wait for it), sMaxUsed (the machine's $30 when known, else the profile's), bedMm {w, h} (jobs must stay on it: the profile's bed, less where $130/$131 are smaller), canStart (everything but the arming and the job's size allows a start), startBlockedBy [plain reasons], lastError | null, warnings [plain], log [the last 50 lines in (\"< \") and out (\"> \")], simStats {burnMm, beamOnTravelMm, beamOnStillMs, movedMm, gcodeLines, overflows, maxRxUsed, resets} | null}",
            always,
            status
        ),
        cmd!(query "laser.home", "Home Laser", [], None, "{} (sends $H: the homing cycle) → laser.status + {sent}", always, home),
        cmd!(
            query "laser.unlock",
            "Unlock Laser",
            [],
            None,
            "{} (sends $X: clears the alarm lock without homing) → laser.status + {sent, warning (the machine no longer knows where it is: home it or check the head's position before cutting)}",
            always,
            unlock
        ),
        cmd!(
            query "laser.jog",
            "Jog Laser",
            [],
            None,
            "{dx?, dy?, dz? (mm, relative: $J=G91) | x?, y?, z? (mm, work coordinates: $J=G90), feed? (mm/min, 1-20000; default the profile's rapid)} (beam off; Z raises or lowers the head: the knife onto the sheet before Set origin; refused while a job runs) → laser.status + {sent}",
            always,
            jog
        ),
        cmd!(query "laser.jogCancel", "Cancel Laser Jog", [], None, "{} (0x85: stops a jog at once) → laser.status", always, jog_cancel),
        cmd!(
            query "laser.setOrigin",
            "Set Laser Origin",
            [],
            None,
            "{} (G10 L20 P1 X0 Y0: the head's position becomes the work origin, where a job's sheet has its bottom-left corner; with the drag knife as the profile's head, G10 L20 P1 X0 Y0 Z0: the blade, lowered until it just touches the sheet, also sets Z 0 at the sheet's surface, which knife jobs and frames wait for on each connection) → laser.status + {sent}",
            always,
            set_origin
        ),
        cmd!(
            query "laser.frame",
            "Frame Laser Job",
            [],
            None,
            "{ids?|id? (the objects to cut, default the selection: any objects, cut along their outlines as laser.gcode cuts them; a single stencil object: its sheet), sheet? (a stencil's sheet, 1-based; needed when it has several), mmPerUnit?, widthMm? (the objects' scale, as laser.gcode), overlapMm?, label? (a stencil's, as modifier.stencilGcode), gcode? (G-code text), path? (a .gcode, .nc, .ngc, .gc, .tap or .txt file), testCard? (card|powerLadder) (at most one of gcode, path and testCard; none: the objects), name? (the job's name), material? (paper|film|card|custom; the saved settings of which material; default the stencil's, else paper), kerfMm?, sMax?, originXMm?, originYMm?, air?, cut?, score?, engrave? ({enabled, speedMmMin, powerPct, passes}: as modifier.stencilGcode; sMax is always the connected machine's $30 when it reports one), powersPct?, speedsMmMin?, squareMm?, gapMm?, passes? (the job, as laser.start), beam? (false, the default: trace the job's bounding box with the beam off, as jogs at the profile's rapid speed; true: with the beam at 1% (M4 S = 1% of $30), which only the desktop app's Laser window can run (armed))} (the machine must be Idle and the box on the bed, as laser.start checks it; with the drag knife: up to the knife's travel height first, never with the beam) → laser.status + {boundsMm, beam, sent, warnings}",
            always,
            frame
        ),
        cmd!(
            query "laser.start",
            "Start Laser Job",
            [],
            None,
            "{ids?|id? (the objects to cut, default the selection: any objects, cut along their outlines as laser.gcode cuts them; a single stencil object: its sheet), sheet? (a stencil's sheet, 1-based; needed when it has several), mmPerUnit?, widthMm? (the objects' scale, as laser.gcode), overlapMm?, label? (a stencil's, as modifier.stencilGcode), gcode? (G-code text), path? (a .gcode, .nc, .ngc, .gc, .tap or .txt file), testCard? (card|powerLadder) (at most one of gcode, path and testCard; none: the objects), name? (the job's name), material? (paper|film|card|custom; the saved settings of which material; default the stencil's, else paper), kerfMm?, sMax?, originXMm?, originYMm?, air?, cut?, score?, engrave? ({enabled, speedMmMin, powerPct, passes}: as modifier.stencilGcode; sMax is always the connected machine's $30 when it reports one), powersPct?, speedsMmMin?, squareMm?, gapMm?, passes? (the power ladder's)} (only the desktop app's Laser window can start a job (armed, after its safety checklist): from anywhere else it is refused; also refused unless the machine is ready and Idle, $32 = 1, and the job stays on the bed: with the profile's origin at the head (the default), the work offset plus the job's bounds must lie in the machine's travel (its sign convention taken from where the head is; when it can't be told, only what can be is checked, with a warning); with the origin at the machine's, the bounds in 0..bed; every line of the G-code (given, read or written by Picabia) must pass the laser allow-list: no $ system commands (settings, homing, jogs, resets), no M3 (constant power), no Z or other axes (PIC35: with the drag knife as the profile's head, jobs are written for the knife, Z is allowed, M3, M4 and S above 0 are not, and Set origin must have zeroed Z on this connection; `head` isn't a job param), no tool, plane, unit (G20: export in mm) or coordinate-system changes (G10, G28, G30, G53, G55-G59, G92), only the G-codes G0-G4, G17, G21, G40, G49, G54, G61, G80, G90, G91, G91.1, G94 and the M-codes M2, M4, M5, M8, M9, M30; the refusal names the line) → laser.status + {started: {name, lines, estimatedSec, boundsMm, warnings}}",
            always,
            start
        ),
        cmd!(query "laser.pause", "Pause Laser", [], None, "{} (feed hold, !: always allowed; nothing happens when not connected; an error when the connected link doesn't confirm within a second: it keeps trying) → laser.status", always, pause),
        cmd!(
            query "laser.resume",
            "Resume Laser",
            [],
            None,
            "{} (~ after a hold; resuming a paused job turns the beam back on, so only the desktop app's Laser window can (armed), as laser.start) → laser.status",
            always,
            resume
        ),
        cmd!(
            query "laser.stop",
            "Stop Laser",
            [],
            None,
            "{} (always allowed, from anywhere: feed hold, then a soft reset once the machine has stopped (at most 0.5 s), then M5; a running job ends stopped; nothing happens when not connected; an error when the connected link doesn't confirm within a second: it keeps trying) → laser.status",
            always,
            stop
        ),
    ]
}

/// The status plus `extra`'s fields.
fn status_with(s: &Session, extra: Value) -> Value {
    let mut v = status_json(s);
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    v
}

fn status(s: &mut Session, _p: &Value) -> Result<Value> {
    sync_machine(s);
    Ok(status_json(s))
}

fn xyz(v: Option<[f64; 3]>) -> Value {
    v.map_or(Value::Null, |p| json!({"x": p[0], "y": p[1], "z": p[2]}))
}

/// `laser.status` (see its params doc).
pub fn status_json(s: &Session) -> Value {
    let link = &s.picabia.laser;
    let prof = Machine::of(s);
    let (bw, bh) = bed_mm(s);
    let mut v = json!({
        "connected": false,
        "phase": "closed",
        "port": Value::Null,
        "portName": Value::Null,
        "sim": false,
        "hardware": link.hardware,
        "state": Value::Null,
        "pos": Value::Null,
        "mpos": Value::Null,
        "feed": Value::Null,
        "power": Value::Null,
        "armed": link.armed,
        "stopping": false,
        "job": Value::Null,
        "alarm": Value::Null,
        "settings": {"sMax": Value::Null, "laserMode": Value::Null, "maxTravel": Value::Null, "homing": Value::Null, "softLimits": Value::Null, "version": Value::Null, "build": Value::Null, "rxSize": vectorcraft_grbl::RX_BUFFER},
        "profile": prof.to_json(),
        "sMaxUsed": link.machine_s_max().unwrap_or(prof.s_max),
        "bedMm": {"w": bw, "h": bh},
        "canStart": false,
        "startBlockedBy": [],
        "lastError": Value::Null,
        "warnings": [],
        "log": [],
        "simStats": Value::Null,
    });
    #[cfg(target_arch = "wasm32")]
    {
        v["startBlockedBy"] = json!([DESKTOP_ONLY]);
        v["warnings"] = json!([DESKTOP_ONLY]);
    }
    #[cfg(not(target_arch = "wasm32"))]
    native::fill_status(link, &prof, &mut v);
    v
}

/// The link, open and ready, or why not.
#[cfg(not(target_arch = "wasm32"))]
fn ready(cmd: &str, s: &Session) -> Result<()> {
    match s.picabia.laser.blocked_by_link() {
        None => Ok(()),
        Some(why) => Err(disabled(cmd, why)),
    }
}

fn ports(s: &mut Session, _p: &Value) -> Result<Value> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        Ok(json!({"ports": []}))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let link = &s.picabia.laser;
        let mut list: Vec<Value> = if link.hardware {
            (link.lister)().into_iter().map(|p| json!({"path": p.path, "name": p.name, "usb": p.usb})).collect()
        } else {
            Vec::new()
        };
        if native::sim_allowed() {
            list.push(json!({"path": SIM_PORT, "name": "Simulated GRBL", "usb": false, "sim": true}));
        }
        Ok(json!({"ports": list}))
    }
}

fn connect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "laser.connect";
    let port = match get(p, "port") {
        Some(Value::String(port)) if !port.trim().is_empty() && port.len() <= 256 && !port.chars().any(char::is_control) => port.trim().to_string(),
        _ => return Err(bad(CMD, "`port` must be a port path from laser.ports")),
    };
    let baud = match get(p, "baud") {
        None => 0,
        Some(v) => match v.as_f64() {
            Some(b) if b.is_finite() && b.fract() == 0.0 && (1200.0..=4_000_000.0).contains(&b) => b as u32,
            _ => return Err(bad(CMD, "`baud` must be a baud rate (1200-4000000)")),
        },
    };
    let sim = get(p, "sim");
    if sim.is_some() && port != SIM_PORT {
        return Err(bad(CMD, "`sim` options go with port \"sim\" only"));
    }
    if sim.is_some_and(|v| !v.is_object()) {
        return Err(bad(CMD, "`sim` must be an object of simulator options"));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, baud);
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Checked before anything is closed or opened: a refused real port leaves the link as it was.
        if port != SIM_PORT && !s.picabia.laser.hardware {
            return Err(disabled(CMD, NO_HARDWARE));
        }
        if s.picabia.laser.job_active() {
            return Err(disabled(CMD, "a job is running: stop it before connecting again"));
        }
        s.picabia.laser.close(SHUTDOWN_BUDGET_MS);
        s.picabia.laser.job = None;
        sync_machine(s);
        s.picabia.laser.open(CMD, &port, baud, sim)?;
        Ok(status_json(s))
    }
}

fn disconnect(s: &mut Session, _p: &Value) -> Result<Value> {
    s.picabia.laser.close(DISCONNECT_BUDGET_MS);
    sync_machine(s);
    Ok(status_json(s))
}

/// Send one-off lines (refused while a job runs).
fn send_lines(cmd: &str, s: &mut Session, lines: &[String]) -> Result<Value> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, lines);
        Err(disabled(cmd, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ready(cmd, s)?;
        s.picabia.laser.command(lines.to_vec()).map_err(|why| disabled(cmd, why))?;
        Ok(json!({"sent": lines}))
    }
}

fn home(s: &mut Session, _p: &Value) -> Result<Value> {
    person_or_sim("laser.home", s)?;
    let r = send_lines("laser.home", s, &["$H".into()])?;
    Ok(status_with(s, r))
}

fn unlock(s: &mut Session, _p: &Value) -> Result<Value> {
    person_or_sim("laser.unlock", s)?;
    let mut r = send_lines("laser.unlock", s, &["$X".into()])?;
    r["warning"] = json!("Unlocked without homing: the machine no longer knows where it is. Home it, or check the head's position, before cutting.");
    Ok(status_with(s, r))
}

fn set_origin(s: &mut Session, _p: &Value) -> Result<Value> {
    person_or_sim("laser.setOrigin", s)?;
    let knife = Machine::of(s).knife();
    let line = if knife { "G10 L20 P1 X0 Y0 Z0" } else { "G10 L20 P1 X0 Y0" };
    let r = send_lines("laser.setOrigin", s, &[line.into()])?;
    #[cfg(not(target_arch = "wasm32"))]
    if knife {
        s.picabia.laser.set_knife_z(true);
    }
    Ok(status_with(s, r))
}

fn jog(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "laser.jog";
    person_or_sim(CMD, s)?;
    let axis = |k: &str| -> Result<Option<f64>> {
        match get(p, k) {
            None => Ok(None),
            Some(_) => match num(p, k) {
                Some(v) if v.abs() <= MAX_JOG_MM => Ok(Some(v)),
                _ => Err(bad(CMD, format!("`{k}` must be a distance in mm (at most {MAX_JOG_MM})"))),
            },
        }
    };
    let (dx, dy, x, y) = (axis("dx")?, axis("dy")?, axis("x")?, axis("y")?);
    let (dz, z) = (axis("dz")?, axis("z")?);
    let relative = dx.is_some() || dy.is_some() || dz.is_some();
    let absolute = x.is_some() || y.is_some() || z.is_some();
    if relative == absolute {
        return Err(bad(CMD, "give dx/dy/dz (a relative jog) or x/y/z (to a work position), not both and not neither"));
    }
    let feed = match get(p, "feed") {
        None => Machine::of(s).rapid_mm_min,
        Some(_) => match num(p, "feed") {
            Some(f) if (1.0..=20_000.0).contains(&f) => f,
            _ => return Err(bad(CMD, "`feed` must be a speed in mm/min (1-20000)")),
        },
    };
    let (mode, ax, ay, az) = if relative { ("G91", dx, dy, dz) } else { ("G90", x, y, z) };
    let mut line = format!("$J={mode} G21");
    if let Some(v) = ax {
        line.push_str(&format!(" X{}", gnum(v)));
    }
    if let Some(v) = ay {
        line.push_str(&format!(" Y{}", gnum(v)));
    }
    if let Some(v) = az {
        line.push_str(&format!(" Z{}", gnum(v)));
    }
    line.push_str(&format!(" F{}", gnum(feed)));
    let r = send_lines(CMD, s, &[line])?;
    Ok(status_with(s, r))
}

fn jog_cancel(s: &mut Session, _p: &Value) -> Result<Value> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        Err(disabled("laser.jogCancel", DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ready("laser.jogCancel", s)?;
        // The link answers at once; one that doesn't is reported by laser.status.
        let _ = s.picabia.laser.act(native::Act::JogCancel);
        Ok(status_json(s))
    }
}

fn pause(s: &mut Session, _p: &Value) -> Result<Value> {
    // Not connected: nothing to pause, and that's fine. Unconfirmed: an error (the flag stays up).
    #[cfg(not(target_arch = "wasm32"))]
    s.picabia.laser.pause_confirmed().map_err(|why| EngineError::Other(format!("laser.pause: {why}")))?;
    Ok(status_json(s))
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    // Not connected: nothing to stop, and that's fine. Unconfirmed: an error (the flag stays up).
    #[cfg(not(target_arch = "wasm32"))]
    s.picabia.laser.stop_confirmed().map_err(|why| EngineError::Other(format!("laser.stop: {why}")))?;
    Ok(status_json(s))
}

fn resume(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "laser.resume";
    // Whatever happens next, the arm is used up.
    let armed = s.picabia.laser.take_arm();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, armed);
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ready(CMD, s)?;
        // A paused job lights the beam again: that takes the person's arm, as a start does.
        if s.picabia.laser.job_active() && !armed {
            return Err(disabled(CMD, NOT_ARMED));
        }
        s.picabia.laser.act(native::Act::Resume).map_err(|why| disabled(CMD, why))?;
        Ok(status_json(s))
    }
}

/// The machine must be Idle, laser mode on, and `bounds` on the bed.
#[cfg(not(target_arch = "wasm32"))]
fn check_machine(cmd: &str, s: &Session, bounds: Rect, need_laser_mode: bool) -> Result<Vec<String>> {
    ready(cmd, s)?;
    if let Some(why) = s.picabia.laser.blocked_by_machine(need_laser_mode) {
        return Err(disabled(cmd, why));
    }
    let prof = Machine::of(s);
    if prof.knife() && !s.picabia.laser.knife_z_set() {
        return Err(disabled(cmd, KNIFE_Z_UNSET));
    }
    let machine = s.picabia.laser.machine_frame();
    check_placement(bounds, bed_mm(s), prof.origin_at_head(), &machine).map_err(|why| disabled(cmd, why))
}

/// What the machine says about where work coordinates sit: per axis (X, Y) the work offset
/// (machine = work + offset), the machine position now, the travel (`$130`/`$131`), and whether
/// homing is on (without it machine coordinates start wherever the head was at power-up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MachineFrame {
    pub wco: Option<[f64; 2]>,
    pub mpos: Option<[f64; 2]>,
    pub travel: [Option<f64>; 2],
    pub homing: bool,
}

/// Whether a job whose G-code reaches `bounds` (work coordinates, mm) fits: never bigger than the
/// `bed`; with the origin at the head (`head`), placed in machine coordinates (work offset plus
/// bounds) within the travel, taking the machine's sign convention from where the head is (GRBL's
/// homed space is often negative: `-travel..0`); when that can't be told, the job must fit one of
/// the two conventions and a warning says only that much was checked; without homing, the work
/// offset or the travel, only the size is checked (with a warning). With the origin at the
/// machine's (`!head`), work coordinates are taken as the bed's: `bounds` within `0..bed`.
/// Returns the warnings, or why not.
pub fn check_placement(bounds: Rect, bed: (f64, f64), head: bool, m: &MachineFrame) -> std::result::Result<Vec<String>, String> {
    let t = BED_TOLERANCE_MM;
    let finite = [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite());
    if !finite {
        return Err("the job has no usable size".into());
    }
    let (w, h) = (bounds.width(), bounds.height());
    if w > bed.0 + t || h > bed.1 + t {
        return Err(format!(
            "the job is {} × {} mm, bigger than the bed ({} × {} mm): use a smaller sheet",
            gnum(w),
            gnum(h),
            gnum(bed.0),
            gnum(bed.1)
        ));
    }
    if !head {
        return check_bed(bounds, bed).map(|()| Vec::new());
    }
    let (Some(wco), true, [Some(tx), Some(ty)]) = (m.wco, m.homing, m.travel) else {
        return Ok(vec![
            "Picabia can't tell where the job sits on the bed (it needs homing, the work offset and the travel $130/$131): only its size was checked. Frame it before starting.".into(),
        ]);
    };
    let mut warnings = Vec::new();
    let axes = [("X", wco[0], tx, bounds.x0, bounds.x1, m.mpos.map(|p| p[0])), ("Y", wco[1], ty, bounds.y0, bounds.y1, m.mpos.map(|p| p[1]))];
    for (axis, off, travel, b0, b1, pos) in axes {
        let (lo, hi) = (off + b0, off + b1);
        let fits_pos = lo >= -t && hi <= travel + t;
        let fits_neg = lo >= -travel - t && hi <= t;
        let sign = pos.filter(|p| p.abs() > 0.5).map(|p| p > 0.0);
        let ok = match sign {
            Some(true) => fits_pos,
            Some(false) => fits_neg,
            None => {
                if fits_pos != fits_neg {
                    warnings.push(format!(
                        "The head is at machine {axis} 0, so Picabia can't tell which way the machine counts {axis}: the job fits one way. Frame it before starting."
                    ));
                }
                fits_pos || fits_neg
            }
        };
        if !ok {
            let range = match sign {
                Some(false) => format!("{} to 0", gnum(-travel)),
                _ => format!("0 to {}", gnum(travel)),
            };
            return Err(format!(
                "the job would reach machine {axis} {} to {}, off the machine's travel ({range} mm): jog to the sheet's bottom-left corner and press Set origin, or use a smaller sheet",
                gnum(lo),
                gnum(hi)
            ));
        }
    }
    Ok(warnings)
}

fn frame(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "laser.frame";
    // A frame uses the arm up whatever happens next.
    let armed = s.picabia.laser.take_arm();
    person_or_sim(CMD, s)?;
    let beam = match get(p, "beam") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return Err(bad(CMD, "`beam` must be true or false")),
    };
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p, armed, beam);
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let prof = Machine::of(s);
        if beam && prof.knife() {
            return Err(disabled(CMD, "the head is the drag knife, which has no beam: frame with the beam off"));
        }
        if beam && !armed {
            return Err(disabled(CMD, NOT_ARMED));
        }
        ready(CMD, s)?;
        let run = build_run(CMD, s, p)?;
        let b = run.bounds;
        let placement = check_machine(CMD, s, b, beam)?;
        let rapid = gnum(Machine::of(s).rapid_mm_min);
        let corners = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1), (b.x0, b.y0)];
        let sent: Vec<String> = if beam {
            let s_max = s.picabia.laser.machine_s_max().unwrap_or(Machine::of(s).s_max);
            let low = (s_max * 0.01).round().max(1.0);
            let mut lines =
                vec!["G21".to_string(), "G90".into(), "M5".into(), format!("G0 X{} Y{}", gnum(b.x0), gnum(b.y0)), format!("M4 S{}", gnum(low))];
            for (i, (x, y)) in corners.iter().enumerate().skip(1) {
                let f = if i == 1 { format!(" F{rapid}") } else { String::new() };
                lines.push(format!("G1 X{} Y{}{f}", gnum(*x), gnum(*y)));
            }
            lines.push("M5".into());
            s.picabia.laser.start_job(format!("Frame of {} (beam at 1%)", run.name), lines.clone()).map_err(|why| disabled(CMD, why))?;
            s.picabia.laser.job = Some(JobInfo {
                source: "frame",
                sheet: run.sheet.clone(),
                layer: run.layer,
                estimated_sec: None,
                bounds: Some(b),
                warnings: Vec::new(),
            });
            lines
        } else {
            let mut lines = vec!["M5".to_string()];
            // The knife goes up to its travel height first, or the frame would drag the blade
            // across the sheet.
            if prof.knife() {
                lines.push(format!("$J=G90 G21 Z{} F{rapid}", gnum(prof.knife.safe_z_mm)));
            }
            lines.extend(corners.iter().map(|(x, y)| format!("$J=G90 G21 X{} Y{} F{rapid}", gnum(*x), gnum(*y))));
            s.picabia.laser.command(lines.clone()).map_err(|why| disabled(CMD, why))?;
            lines
        };
        Ok(status_with(s, json!({"boundsMm": rect_json(b), "beam": beam, "sent": sent, "warnings": placement})))
    }
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "laser.start";
    // The attempt uses the arm up, whether it starts or not.
    let armed = s.picabia.laser.take_arm();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p, armed);
        Err(disabled(CMD, DESKTOP_ONLY))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !armed {
            return Err(disabled(CMD, NOT_ARMED));
        }
        ready(CMD, s)?;
        let mut run: LaserRun = build_run(CMD, s, p)?;
        let placement = check_machine(CMD, s, run.bounds, true)?;
        run.warnings.extend(placement);
        // The lines were cleaned and checked here, before the worker sees them.
        let lines = std::mem::take(&mut run.lines);
        let count = lines.len();
        s.picabia.laser.start_job(run.name.clone(), lines).map_err(|why| disabled(CMD, why))?;
        let started =
            json!({"name": run.name, "lines": count, "estimatedSec": run.estimated_sec, "boundsMm": rect_json(run.bounds), "warnings": run.warnings});
        s.picabia.laser.job = Some(JobInfo {
            source: run.source,
            sheet: run.sheet,
            layer: run.layer,
            estimated_sec: run.estimated_sec,
            bounds: Some(run.bounds),
            warnings: run.warnings,
        });
        Ok(status_with(s, json!({"started": started})))
    }
}
