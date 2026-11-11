//! PICABIA: the laser link's worker thread and the port it owns (desktop builds only).
//!
//! The worker loop, every iteration: the stop and pause flags first (any thread may raise them
//! through a [`LaserStopHandle`]), then the commands' messages, then one I/O step that writes what
//! the controller wants (a stop's `!` included) *before* it reads, reads once, and writes again.
//! A port error mid-job tries a last-resort hold and reset before the port is dropped; a write that
//! only timed out keeps the link (after the hold) and stops the job. A panic in the worker is
//! caught: the hold and reset go out, and the link reports itself closed with the reason.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_grbl::sim::{SimGrbl, SimOptions, SimStats, SimTransport};
use vectorcraft_grbl::{Controller, HOLD, JobState, MachineState, Phase, RESET, Snapshot, Transport};

use super::{LaserLink, Machine, NO_HARDWARE, SHUTDOWN_BUDGET_MS, SIM_PORT, STATUS_LOG_LINES, xyz};
use crate::{EngineError, Result};

/// What the worker publishes at most this often while nothing changes (ms).
const PUBLISH_MS: u64 = 10;
/// How long a command waits for the worker's answer.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);
/// A start the worker sees later than this after it was asked for is dropped (the caller has given
/// up waiting by then, or is about to).
const START_DEADLINE: Duration = Duration::from_millis(700);
/// Consecutive write timeouts a live port is given (each followed by a hold) before it counts as
/// lost.
const MAX_WRITE_TIMEOUTS: u32 = 3;

/// The simulator is a port in debug builds, or with `PICABIA_LASER_SIM=1`.
pub fn sim_allowed() -> bool {
    cfg!(debug_assertions) || std::env::var("PICABIA_LASER_SIM").is_ok_and(|v| v.trim() == "1")
}

/// The worker's answer: done, or refused (the reason).
type Reply = Sender<std::result::Result<(), String>>;

/// A realtime action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Resume,
    JogCancel,
}

/// A message to the worker. Answers go out after the new state is published, so a command returns
/// the state its message made.
pub enum Msg {
    /// One-off lines.
    Command(Vec<String>, Reply),
    /// Start a job, unless the worker sees this after `deadline` (the caller gave up: the start is
    /// dropped, never run).
    Start { name: String, lines: Vec<String>, deadline: Instant, reply: Reply },
    /// A realtime action, answered.
    Act(Act, Reply),
    /// Stop a running job (waiting at most `budget_ms` for the stop sequence), close the port, end
    /// the worker.
    Close { budget_ms: u64 },
}

/// The flags any thread may raise, checked first on every turn of the worker's loop, and the
/// worker's count of the ones it acted on.
#[derive(Debug, Default)]
pub struct Flags {
    stop: AtomicBool,
    pause: AtomicBool,
    stops_done: AtomicU64,
    pauses_done: AtomicU64,
}

/// Stops the laser link's job or motion from any thread, without the session (the UI's always-
/// visible Stop, a keyboard shortcut handler, a watchdog). It belongs to one connection: after a
/// reconnect, take a new one.
#[derive(Clone, Debug)]
pub struct LaserStopHandle {
    flags: Arc<Flags>,
}

impl LaserStopHandle {
    /// Ask the worker to stop (hold, reset, `M5`). Never blocks; the worker acts on it before
    /// anything else on its next turn (within a serial read, at most about 20 ms).
    pub fn stop(&self) {
        self.flags.stop.store(true, Ordering::SeqCst);
    }

    /// Stops the worker has acted on so far.
    pub fn stops_done(&self) -> u64 {
        self.flags.stops_done.load(Ordering::SeqCst)
    }
}

/// The worker's state as the commands see it.
#[derive(Default)]
pub struct Shared {
    pub snap: Snapshot,
    /// The worker's clock (ms since the port opened): the job's times are on it.
    pub now_ms: u64,
    pub sim: Option<SimStats>,
    /// The worker is running.
    pub running: bool,
    /// The link was closed: whatever the worker still publishes, it no longer counts as running.
    pub closed: bool,
    /// Why the port went away (or the worker failed).
    pub io_error: Option<String>,
}

/// The port the worker owns.
pub trait LinkPort: Transport {
    fn sim_stats(&self) -> Option<SimStats> {
        None
    }
}

impl LinkPort for vectorcraft_grbl::serial::SerialTransport {}

/// The simulator as a port: a real clock (sped up `time_scale` times), a short sleep when nothing
/// arrived (a serial read waits for bytes; the simulator answers at once), an optional pulled
/// cable, write timeouts and (tests) a worker panic.
struct SimPort {
    inner: SimTransport,
    disconnect_at_line: Option<usize>,
    /// Writes time out (once each) from this G-code line on, this many times.
    timeout_at_line: Option<usize>,
    timeouts_left: u32,
    /// Tests: the worker panics once the machine has read this many G-code lines.
    #[cfg_attr(not(test), allow(dead_code))]
    panic_at_line: Option<usize>,
    time_scale: u64,
}

impl Transport for SimPort {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.cable()?;
        // A realtime-only write (the hold and reset) goes through; a timed-out line doesn't.
        let realtime = data.iter().all(|b| matches!(*b, HOLD | RESET | b'?' | b'~' | 0x85));
        if !realtime
            && self.timeouts_left > 0
            && let Some(n) = self.timeout_at_line
            && self.inner.sim().stats().gcode_lines >= n
        {
            self.timeouts_left -= 1;
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the simulated write timed out"));
        }
        self.inner.write(data)
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.cable()?;
        #[cfg(test)]
        if let Some(n) = self.panic_at_line
            && self.inner.sim().stats().gcode_lines >= n
        {
            panic!("simulated worker panic");
        }
        let n = self.inner.read(buf)?;
        if n == 0 {
            let sim = self.inner.sim();
            let busy = sim.state() != MachineState::Idle || sim.rx_used() > 0 || sim.planner_len() > 0;
            // A sped-up clock answers within microseconds: yield rather than sleep (a short sleep
            // lasts about a millisecond on some systems). At real time, wait one simulated
            // millisecond.
            if busy && self.time_scale >= 10 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_micros(if busy { 1000 / self.time_scale.max(1) } else { 2000 }));
            }
        }
        Ok(n)
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

impl SimPort {
    fn cable(&self) -> std::io::Result<()> {
        match self.disconnect_at_line {
            Some(n) if self.inner.sim().stats().gcode_lines >= n => {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the simulated cable was pulled"))
            }
            _ => Ok(()),
        }
    }
}

impl LinkPort for SimPort {
    fn sim_stats(&self) -> Option<SimStats> {
        Some(self.inner.sim().stats().clone())
    }
}

/// The simulator's options from `laser.connect {sim}` (lenient: wrong types are ignored).
fn sim_port(v: Option<&Value>, clock: Box<dyn FnMut() -> u64 + Send>, time_scale: &mut u64) -> SimPort {
    let mut o = SimOptions::default();
    let m = v.and_then(Value::as_object);
    let num = |k: &str| m.and_then(|m| m.get(k)).and_then(Value::as_f64).filter(|x| x.is_finite());
    let line = |k: &str| num(k).filter(|x| *x >= 1.0 && *x <= 1e9).map(|x| x as usize);
    let flag = |k: &str| m.and_then(|m| m.get(k)).and_then(Value::as_bool);
    if let Some(b) = flag("laserMode") {
        o.laser_mode = b;
    }
    if let Some(x) = num("sMax") {
        o.s_max = x.clamp(1.0, 100_000.0);
    }
    if let Some(a) = m.and_then(|m| m.get("maxTravelMm")).and_then(Value::as_array) {
        for (i, t) in a.iter().take(3).enumerate() {
            if let (Some(t), Some(slot)) = (t.as_f64().filter(|x| x.is_finite()), o.max_travel.get_mut(i)) {
                *slot = t.clamp(1.0, 10_000.0);
            }
        }
    }
    if let Some(x) = num("speedFactor") {
        o.speed_factor = x.clamp(0.01, 1e6);
    }
    if let Some(x) = num("rapidMmMin") {
        o.rapid_mm_min = x.clamp(1.0, 100_000.0);
    }
    o.alarm_at_line = line("alarmAtLine");
    o.error_at_line = line("errorAtLine");
    if let Some(b) = flag("homing") {
        o.homing = b;
    }
    if let Some(b) = flag("startInAlarm") {
        o.start_in_alarm = b;
    }
    if let Some(b) = flag("bannerOnOpen") {
        o.banner_on_open = b;
    }
    *time_scale = num("timeScale").map_or(1, |x| x.clamp(1.0, 1000.0) as u64);
    let timeouts = num("writeTimeouts").map_or(1, |x| x.clamp(1.0, 100.0) as u32);
    SimPort {
        inner: SimTransport::new(SimGrbl::new(o), clock),
        disconnect_at_line: line("disconnectAtLine"),
        timeout_at_line: line("writeTimeoutAtLine"),
        timeouts_left: timeouts,
        // Only the engine's tests may make the worker panic.
        panic_at_line: if cfg!(test) { line("panicAtLine") } else { None },
        time_scale: *time_scale,
    }
}

/// The running worker (or, after a close that timed out, the one still ending).
pub struct Worker {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Shared>>,
    flags: Arc<Flags>,
    handle: Option<JoinHandle<()>>,
    pub port: String,
    pub port_name: String,
    pub sim: bool,
    /// PICABIA (PIC35): Set origin zeroed Z with the knife as the head on this connection. GRBL
    /// keeps a work offset across connections and power cycles, so without this a knife job's
    /// `G0 Z5` could be 5 mm above wherever an old offset put Z 0, not above the sheet.
    pub knife_z_set: bool,
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Drop for LaserLink {
    fn drop(&mut self) {
        self.close(SHUTDOWN_BUDGET_MS);
    }
}

/// Open a real serial port. The engine's own tests never reach a device: there this always
/// refuses.
#[cfg(not(test))]
fn open_serial(cmd: &str, port: &str, baud: u32) -> Result<Box<dyn LinkPort>> {
    let t = vectorcraft_grbl::serial::open(port, baud).map_err(|e| EngineError::Other(format!("{cmd}: couldn't open {port}: {e}")))?;
    Ok(Box::new(t))
}

#[cfg(test)]
fn open_serial(cmd: &str, _port: &str, _baud: u32) -> Result<Box<dyn LinkPort>> {
    Err(EngineError::Disabled(cmd.into(), "the engine's tests never open a real port".into()))
}

impl LaserLink {
    /// Open `port` and start the worker.
    pub(super) fn open(&mut self, cmd: &str, port: &str, baud: u32, sim: Option<&Value>) -> Result<()> {
        // A worker whose close timed out may still hold the port: never race it for it.
        if self.worker.as_ref().and_then(|w| w.handle.as_ref()).is_some_and(|h| !h.is_finished()) {
            return Err(EngineError::Disabled(cmd.into(), "the previous laser connection is still closing; try again in a moment".into()));
        }
        let (link, name, is_sim): (Box<dyn LinkPort>, String, bool) = if port == SIM_PORT {
            if !sim_allowed() {
                return Err(EngineError::Disabled(cmd.into(), "the simulator is available in debug builds or with PICABIA_LASER_SIM=1".into()));
            }
            let t0 = Instant::now();
            let scale = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
            let clock_scale = scale.clone();
            let clock = Box::new(move || {
                u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX).saturating_mul(clock_scale.load(Ordering::Relaxed)) / 1000
            });
            let mut time_scale = 1;
            let p = sim_port(sim, clock, &mut time_scale);
            scale.store(time_scale, Ordering::Relaxed);
            (Box::new(p), "Simulated GRBL".into(), true)
        } else {
            // Real ports only once the desktop app has enabled them (checked again here, next to
            // the open, whatever the caller checked).
            if !self.hardware {
                return Err(EngineError::Disabled(cmd.into(), NO_HARDWARE.into()));
            }
            let t = open_serial(cmd, port, baud)?;
            let name = (self.lister)().into_iter().find(|p| p.path == port).map_or_else(|| port.to_string(), |p| p.name);
            (t, name, false)
        };
        let shared = Arc::new(Mutex::new(Shared { running: true, ..Shared::default() }));
        let flags = Arc::new(Flags::default());
        let (tx, rx) = channel();
        let (worker_shared, worker_flags) = (shared.clone(), flags.clone());
        let handle = std::thread::Builder::new()
            .name("picabia-laser".into())
            .spawn(move || guarded_run(link, rx, worker_shared, worker_flags))
            .map_err(|e| EngineError::Other(format!("{cmd}: couldn't start the laser link: {e}")))?;
        self.worker =
            Some(Worker { tx, shared, flags, handle: Some(handle), port: port.to_string(), port_name: name, sim: is_sim, knife_z_set: false });
        Ok(())
    }

    /// The open link is a real machine, not the simulator.
    pub(super) fn on_real_machine(&self) -> bool {
        #[cfg(test)]
        if self.pretend_real && self.is_open() {
            return true;
        }
        self.is_open() && self.worker.as_ref().is_some_and(|w| !w.sim)
    }

    /// The worker is running.
    pub fn is_open(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| {
            let sh = lock(&w.shared);
            sh.running && !sh.closed
        })
    }

    /// A stop handle for the open connection.
    pub(super) fn stop_handle(&self) -> Option<LaserStopHandle> {
        self.worker.as_ref().filter(|_| self.is_open()).map(|w| LaserStopHandle { flags: w.flags.clone() })
    }

    /// Read the shared state.
    fn with<R>(&self, f: impl FnOnce(&Shared) -> R) -> Option<R> {
        self.worker.as_ref().map(|w| f(&lock(&w.shared)))
    }

    /// Send `msg` to a running worker (false: there is none).
    fn post(&self, msg: Msg) -> bool {
        self.is_open() && self.worker.as_ref().is_some_and(|w| w.tx.send(msg).is_ok())
    }

    fn ask(&self, msg: impl FnOnce(Reply) -> Msg) -> std::result::Result<(), String> {
        let (tx, rx) = channel();
        if !self.post(msg(tx)) {
            return Err("the laser isn't connected".into());
        }
        rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| Err("the laser link didn't answer".into()))
    }

    /// Queue one-off lines.
    pub(super) fn command(&self, lines: Vec<String>) -> std::result::Result<(), String> {
        self.ask(|reply| Msg::Command(lines, reply))
    }

    /// Start streaming a job (lines already cleaned and checked). The worker drops a start it
    /// sees after [`START_DEADLINE`]; a start the caller stopped waiting for is stopped as well, so
    /// a job never runs after its caller was told it failed.
    pub(super) fn start_job(&self, name: String, lines: Vec<String>) -> std::result::Result<(), String> {
        let deadline = Instant::now() + START_DEADLINE;
        let r = self.ask(|reply| Msg::Start { name, lines, deadline, reply });
        if r.as_ref().is_err_and(|e| e == "the laser link didn't answer")
            && let Some(w) = &self.worker
        {
            w.flags.stop.store(true, Ordering::SeqCst);
        }
        r
    }

    /// Do `a` and wait (at most a second) until it is done.
    pub(super) fn act(&self, a: Act) -> std::result::Result<(), String> {
        self.ask(|reply| Msg::Act(a, reply))
    }

    /// Raise the stop (or pause) flag and wait (at most a second) for the worker to act on it.
    /// `Ok(false)`: not connected, nothing to do. An error: the worker didn't confirm; the flag
    /// stays raised, so it acts on it as soon as it can.
    fn flag(&self, stop: bool) -> std::result::Result<bool, String> {
        let Some(w) = self.worker.as_ref().filter(|_| self.is_open()) else { return Ok(false) };
        let (flag, done) = if stop { (&w.flags.stop, &w.flags.stops_done) } else { (&w.flags.pause, &w.flags.pauses_done) };
        let before = done.load(Ordering::SeqCst);
        flag.store(true, Ordering::SeqCst);
        let end = Instant::now() + REPLY_TIMEOUT;
        while Instant::now() < end {
            if done.load(Ordering::SeqCst) > before {
                // Give the worker a moment to publish what the stop made.
                let t = Instant::now() + Duration::from_millis(50);
                while Instant::now() < t && self.with(|sh| sh.snap.job.as_ref().is_some_and(|j| j.state == JobState::Running)).unwrap_or(false) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                return Ok(true);
            }
            if !self.is_open() {
                // The link ended meanwhile (its close stops the machine).
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(format!("the laser link didn't confirm the {}; it will keep trying", if stop { "stop" } else { "pause" }))
    }

    /// Stop a running job or motion and wait for the worker to confirm.
    pub(super) fn stop_confirmed(&self) -> std::result::Result<bool, String> {
        self.flag(true)
    }

    /// Pause (feed hold) and wait for the worker to confirm.
    pub(super) fn pause_confirmed(&self) -> std::result::Result<bool, String> {
        self.flag(false)
    }

    /// Stop a running job or motion (never blocks).
    pub(super) fn stop(&mut self) {
        if let Some(w) = &self.worker {
            w.flags.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Stop what runs (waiting at most about `budget_ms`), close the port, end the worker. A worker
    /// that doesn't end in time is marked closed (it can no longer report itself running) and kept,
    /// so a reconnect waits for it to let go of the port.
    pub(super) fn close(&mut self, budget_ms: u64) {
        let Some(mut w) = self.worker.take() else { return };
        w.flags.stop.store(true, Ordering::SeqCst);
        // A worker that already ended has dropped its receiver: nothing to tell it.
        let _ = w.tx.send(Msg::Close { budget_ms });
        let deadline = Instant::now() + Duration::from_millis(budget_ms + 150);
        if let Some(h) = w.handle.take() {
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            if h.is_finished() {
                // A panic in the worker was caught inside it; nothing is left to clean up here.
                let _ = h.join();
            } else {
                w.handle = Some(h);
            }
        }
        // Keep the last state for laser.status (phase, the job's end, the log).
        {
            let mut sh = lock(&w.shared);
            sh.closed = true;
            sh.running = false;
        }
        self.worker = Some(w);
    }

    /// A job is running or paused.
    pub(super) fn job_active(&self) -> bool {
        self.is_open() && self.with(|sh| sh.snap.job.as_ref().is_some_and(|j| j.state.is_active())).unwrap_or(false)
    }

    /// The connected machine's `$30`.
    pub(crate) fn machine_s_max(&self) -> Option<f64> {
        if !self.is_open() {
            return None;
        }
        self.with(|sh| sh.snap.settings.s_max().filter(|v| v.is_finite() && *v > 0.0)).flatten()
    }

    /// The connected machine's `$130`, `$131`.
    pub(crate) fn machine_travel(&self) -> (Option<f64>, Option<f64>) {
        if !self.is_open() {
            return (None, None);
        }
        self.with(|sh| (sh.snap.settings.get(130), sh.snap.settings.get(131))).unwrap_or((None, None))
    }

    /// Where work coordinates sit on the machine (see [`super::MachineFrame`]).
    pub(crate) fn machine_frame(&self) -> super::MachineFrame {
        if !self.is_open() {
            return super::MachineFrame::default();
        }
        self.with(|sh| {
            let st = sh.snap.status.as_ref();
            let mpos = st.and_then(|s| s.mpos);
            let wpos = st.and_then(|s| s.wpos);
            let wco = st.and_then(|s| s.wco).or_else(|| mpos.zip(wpos).map(|(m, w)| [m[0] - w[0], m[1] - w[1], m[2] - w[2]]));
            let travel = |n: u16| sh.snap.settings.get(n).filter(|t| t.is_finite() && *t > 0.0);
            super::MachineFrame {
                wco: wco.map(|w| [w[0], w[1]]),
                mpos: mpos.map(|m| [m[0], m[1]]),
                travel: [travel(130), travel(131)],
                homing: sh.snap.settings.homing() == Some(true),
            }
        })
        .unwrap_or_default()
    }

    /// Why nothing can be sent now (not connected, still connecting, stopping).
    pub(super) fn blocked_by_link(&self) -> Option<String> {
        if !self.is_open() {
            return Some(match self.with(|sh| sh.io_error.clone()).flatten() {
                Some(e) => format!("the laser isn't connected ({e})"),
                None => "the laser isn't connected".into(),
            });
        }
        self.with(|sh| {
            if sh.snap.phase != Phase::Ready {
                Some("the laser is still connecting (reading its settings)".to_string())
            } else if sh.snap.stopping {
                Some("the laser is stopping; wait until it is back".to_string())
            } else {
                None
            }
        })
        .flatten()
    }

    /// Set origin zeroed Z for the knife on this connection (see [`Worker::knife_z_set`]).
    pub(super) fn knife_z_set(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| w.knife_z_set)
    }

    pub(super) fn set_knife_z(&mut self, set: bool) {
        if let Some(w) = self.worker.as_mut() {
            w.knife_z_set = set;
        }
    }

    /// Why a job or frame can't start now on the machine's side.
    pub(super) fn blocked_by_machine(&self, need_laser_mode: bool) -> Option<String> {
        self.with(|sh| machine_blocks(&sh.snap, need_laser_mode).into_iter().next()).flatten()
    }
}

/// Everything on the machine's side that keeps a job from starting.
fn machine_blocks(snap: &Snapshot, need_laser_mode: bool) -> Vec<String> {
    let mut why = Vec::new();
    if snap.job.as_ref().is_some_and(|j| j.state.is_active()) {
        why.push("a job is running: pause or stop it first".to_string());
    }
    let state = snap.state();
    if state != MachineState::Idle {
        why.push(match state {
            MachineState::Alarm => "the machine is in alarm: home it (or unlock it) first".to_string(),
            MachineState::Unknown => "the machine hasn't reported its state yet".to_string(),
            s => format!("the machine must be Idle to start (it is {})", s.as_str()),
        });
    }
    if need_laser_mode {
        match snap.settings.laser_mode() {
            Some(true) => {}
            Some(false) => why.push(vectorcraft_grbl::Refusal::LaserModeOff.to_string()),
            None => why.push(vectorcraft_grbl::Refusal::LaserModeUnknown.to_string()),
        }
    }
    why
}

/// Fill `laser.status` from the worker's state.
pub(super) fn fill_status(link: &LaserLink, prof: &Machine, v: &mut Value) {
    let Some(w) = link.worker.as_ref() else {
        v["startBlockedBy"] = json!(["the laser isn't connected"]);
        return;
    };
    let sh = lock(&w.shared);
    let snap = &sh.snap;
    let open = sh.running && !sh.closed;
    let phase = if open || snap.phase == Phase::Lost { snap.phase.as_str() } else { "closed" };
    v["connected"] = json!(open && matches!(snap.phase, Phase::Connecting | Phase::Ready));
    v["phase"] = json!(phase);
    v["port"] = json!(w.port);
    v["portName"] = json!(w.port_name);
    v["sim"] = json!(w.sim);
    if let Some(st) = &snap.status {
        v["state"] = json!(st.state.as_str());
        v["pos"] = xyz(st.wpos);
        v["mpos"] = xyz(st.mpos);
        v["feed"] = json!(st.feed);
        v["power"] = json!(st.spindle);
    }
    v["stopping"] = json!(snap.stopping);
    if let Some(a) = snap.alarm {
        v["alarm"] = json!({"code": a.code, "text": a.text, "advice": a.advice});
    }
    let set = &snap.settings;
    v["settings"] = json!({
        "sMax": set.s_max(),
        "laserMode": set.laser_mode(),
        "maxTravel": set.max_travel(),
        "homing": set.homing(),
        "softLimits": set.soft_limits(),
        "version": snap.version,
        "build": snap.build,
        "rxSize": snap.rx_size,
    });
    if let Some(j) = &snap.job {
        let info = link.job.clone().unwrap_or_default();
        let end = j.ended_ms.unwrap_or(sh.now_ms);
        let elapsed = end.saturating_sub(j.started_ms) as f64 / 1000.0;
        let percent = if j.lines == 0 { 0.0 } else { j.acked as f64 * 100.0 / j.lines as f64 };
        let percent = if j.state == JobState::Done { 100.0 } else { (percent * 10.0).round() / 10.0 };
        let eta = if !j.state.is_active() {
            Some(0.0)
        } else if percent >= 5.0 {
            Some(elapsed * (100.0 - percent) / percent)
        } else {
            info.estimated_sec.map(|e| (e - elapsed).max(0.0))
        };
        let mut job = json!({
            "name": j.name,
            "state": j.state.as_str(),
            "line": j.acked,
            "lines": j.lines,
            "sent": j.sent,
            "percent": percent,
            "elapsedSec": (elapsed * 10.0).round() / 10.0,
            "etaSec": eta.map(|e| (e * 10.0).round() / 10.0),
            "estimatedSec": info.estimated_sec,
            "source": if info.source.is_empty() { "gcode" } else { info.source },
            "sheet": info.sheet,
            "layer": info.layer.map(|id| json!(id.0)),
            "boundsMm": info.bounds.map_or(Value::Null, super::rect_json),
            "warnings": info.warnings,
        });
        if let JobState::Failed { line, why } = &j.state {
            job["why"] = json!(why);
            job["failedLine"] = json!(line);
        }
        v["job"] = job;
    }
    v["lastError"] = json!(snap.last_error.clone().or_else(|| sh.io_error.clone()));
    let skip = snap.log.len().saturating_sub(STATUS_LOG_LINES);
    v["log"] = json!(snap.log.iter().skip(skip).collect::<Vec<_>>());
    if let Some(st) = &sh.sim {
        v["simStats"] = json!({
            "burnMm": st.burn_mm,
            "beamOnTravelMm": st.beam_on_travel_mm,
            "beamOnStillMs": st.beam_on_still_ms,
            "movedMm": st.moved_mm,
            "gcodeLines": st.gcode_lines,
            "overflows": st.overflows,
            "maxRxUsed": st.max_rx_used,
            "resets": st.resets,
        });
    }
    // What blocks a start, and what to know.
    let mut blocked = Vec::new();
    if !open {
        blocked.push("the laser isn't connected".to_string());
    } else if snap.phase != Phase::Ready {
        blocked.push("the laser is still connecting (reading its settings)".to_string());
    } else {
        if snap.stopping {
            blocked.push("the laser is stopping".to_string());
        }
        blocked.extend(machine_blocks(snap, true));
        if prof.knife() && !w.knife_z_set {
            blocked.push(super::KNIFE_Z_UNSET.to_string());
        }
    }
    v["knifeZSet"] = json!(w.knife_z_set);
    v["canStart"] = json!(blocked.is_empty());
    v["startBlockedBy"] = json!(blocked);
    let mut warnings = Vec::new();
    if open && snap.phase == Phase::Ready {
        match set.laser_mode() {
            Some(false) => warnings.push(vectorcraft_grbl::Refusal::LaserModeOff.to_string()),
            None => warnings.push(vectorcraft_grbl::Refusal::LaserModeUnknown.to_string()),
            Some(true) => {}
        }
        if let Some(m) = set.s_max()
            && (m - prof.s_max).abs() > 1e-9
        {
            warnings.push(format!("The machine's $30 is {m}; the profile says {}: jobs use the machine's.", prof.s_max));
        }
        for (axis, n, bed) in [("X", 130, prof.bed_w_mm), ("Y", 131, prof.bed_h_mm)] {
            if let Some(t) = set.get(n)
                && t > 0.0
                && t + super::BED_TOLERANCE_MM < bed
            {
                warnings.push(format!(
                    "The machine's {axis} travel (${n} = {t} mm) is less than the profile's bed ({bed} mm): jobs must fit the travel."
                ));
            }
        }
    }
    if !prof.checked {
        warnings.push("The machine profile's values are guesses until checked on the machine.".to_string());
    }
    v["warnings"] = json!(warnings);
}

/// [`run`] with a panic caught: the last-resort hold and reset go out, and the link reports itself
/// closed with the reason instead of claiming to run.
fn guarded_run(mut port: Box<dyn LinkPort>, rx: Receiver<Msg>, shared: Arc<Mutex<Shared>>, flags: Arc<Flags>) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(port.as_mut(), &rx, &shared, &flags)));
    if r.is_err() {
        // The beam off before anything else; a port that fails now is gone already.
        let _ = port.write(&[HOLD, RESET]);
        let mut sh = lock(&shared);
        sh.running = false;
        sh.io_error = Some("the laser link failed (an internal error); the machine was sent a hold and reset".into());
        if let Some(j) = sh.snap.job.as_mut().filter(|j| j.state.is_active()) {
            j.state = JobState::Failed { line: j.acked, why: "the laser link failed (an internal error)".into() };
        }
        // Tells the commands waiting on the stop flag that nothing more will run.
        flags.stops_done.fetch_add(1, Ordering::SeqCst);
    }
}

/// The worker: the flags first, then messages, then one I/O step; publish the state.
fn run(port: &mut dyn LinkPort, rx: &Receiver<Msg>, shared: &Mutex<Shared>, flags: &Flags) {
    let t0 = Instant::now();
    let now = move || u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut ctrl = Controller::new();
    ctrl.on_connected(now());
    let mut last_key = None;
    let mut last_pub = 0u64;
    let mut replies: Vec<(Reply, std::result::Result<(), String>)> = Vec::new();
    let mut buf = vec![0u8; 4096];
    let mut timeouts = 0u32;
    loop {
        let mut changed = false;
        // The stop and pause flags before anything else, written out at once.
        let stop = flags.stop.swap(false, Ordering::SeqCst);
        let pause = flags.pause.swap(false, Ordering::SeqCst);
        if stop || pause {
            if stop {
                ctrl.stop();
            } else {
                ctrl.pause();
            }
            let out = ctrl.poll(now());
            if !out.is_empty() && port.write(&out).is_err() {
                // The last resort: hold and reset straight away.
                let _ = port.write(&[HOLD, RESET]);
            }
            publish(&ctrl, port, shared, now(), true, None);
            if stop {
                flags.stops_done.fetch_add(1, Ordering::SeqCst);
            } else {
                flags.pauses_done.fetch_add(1, Ordering::SeqCst);
            }
        }
        loop {
            match rx.try_recv() {
                Ok(Msg::Close { budget_ms }) => {
                    close(&mut ctrl, port, budget_ms, &now);
                    publish(&ctrl, port, shared, now(), false, None);
                    return;
                }
                Ok(m) => {
                    replies.extend(handle(&mut ctrl, m));
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    close(&mut ctrl, port, SHUTDOWN_BUDGET_MS, &now);
                    publish(&ctrl, port, shared, now(), false, None);
                    return;
                }
            }
        }
        if changed {
            publish(&ctrl, port, shared, now(), true, None);
            for (reply, r) in replies.drain(..) {
                // A command that gave up waiting has dropped its receiver.
                let _ = reply.send(r);
            }
        }
        match step(&mut ctrl, port, &mut buf, now()) {
            Ok(()) => timeouts = 0,
            Err(e) => {
                // Before anything else: hold and reset, the beam off.
                let held = port.write(&[HOLD, RESET]).is_ok();
                let job_active = ctrl.job().is_some_and(|j| j.state.is_active());
                if held && e.kind() == std::io::ErrorKind::TimedOut && timeouts < MAX_WRITE_TIMEOUTS {
                    // A live port that only timed out: the link stays, the job stops (the reset
                    // purged what the machine had).
                    timeouts += 1;
                    if job_active {
                        ctrl.stop();
                    }
                    publish(&ctrl, port, shared, now(), true, Some(format!("a write to the laser timed out ({e}); the job was stopped")));
                    continue;
                }
                ctrl.on_disconnect();
                publish(&ctrl, port, shared, now(), false, Some(format!("the port failed: {e}")));
                return;
            }
        }
        if ctrl.phase() == Phase::Lost {
            publish(&ctrl, port, shared, now(), false, None);
            return;
        }
        let t = now();
        let key = (ctrl.phase(), ctrl.job().map(|j| j.state.clone()));
        if last_key.as_ref() != Some(&key) || t >= last_pub + PUBLISH_MS {
            publish(&ctrl, port, shared, t, true, None);
            last_key = Some(key);
            last_pub = t;
        }
    }
}

fn publish(ctrl: &Controller, port: &dyn LinkPort, shared: &Mutex<Shared>, now_ms: u64, running: bool, io_error: Option<String>) {
    let snap = ctrl.snapshot();
    let sim = port.sim_stats();
    let mut sh = lock(shared);
    sh.snap = snap;
    sh.now_ms = now_ms;
    sh.sim = sim;
    // A closed link stays closed, whatever a late worker publishes.
    sh.running = running && !sh.closed;
    if io_error.is_some() {
        sh.io_error = io_error;
    }
}

/// Write what the controller wants now.
fn flush(ctrl: &mut Controller, port: &mut dyn LinkPort, now_ms: u64) -> std::io::Result<()> {
    let out = ctrl.poll(now_ms);
    if out.is_empty() { Ok(()) } else { port.write(&out) }
}

/// One I/O step: write what is waiting (a stop's `!` goes out before the read, not after it), one
/// read (a serial read returns as soon as bytes arrive, else after its 20 ms timeout), then write
/// what the answer calls for at once. (Reading until a read comes back empty would wait out the
/// timeout after every answer before refilling the machine's buffer.)
fn step(ctrl: &mut Controller, port: &mut dyn LinkPort, buf: &mut [u8], now_ms: u64) -> std::io::Result<()> {
    flush(ctrl, port, now_ms)?;
    let n = port.read(buf)?;
    if let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) {
        ctrl.on_bytes(chunk, now_ms);
    }
    flush(ctrl, port, now_ms)
}

/// Do what `msg` asks; the answer to send once the state is published.
pub(super) fn handle(ctrl: &mut Controller, msg: Msg) -> Option<(Reply, std::result::Result<(), String>)> {
    match msg {
        Msg::Command(lines, reply) => {
            let mut r = Ok(());
            for line in &lines {
                if let Err(e) = ctrl.command(line) {
                    r = Err(e.to_string());
                    break;
                }
            }
            Some((reply, r))
        }
        Msg::Start { name, lines, deadline, reply } => {
            // The caller has given up (or is about to): never start a job it was told failed.
            if Instant::now() > deadline {
                return Some((reply, Err("the start came too late and was dropped".into())));
            }
            Some((reply, ctrl.start_job(&name, lines).map_err(|e| e.to_string())))
        }
        Msg::Act(a, reply) => {
            match a {
                Act::Resume => ctrl.resume(),
                Act::JogCancel => ctrl.jog_cancel(),
            }
            Some((reply, Ok(())))
        }
        // Handled by the loop.
        Msg::Close { .. } => None,
    }
}

/// Stop what the machine is doing before the port closes: the stop sequence, given at most
/// `budget_ms`; if it hasn't finished, hold and reset outright (the reset turns the laser off).
fn close(ctrl: &mut Controller, port: &mut dyn LinkPort, budget_ms: u64, now: &dyn Fn() -> u64) {
    let snap = ctrl.snapshot();
    let moving = snap.job.as_ref().is_some_and(|j| j.state.is_active())
        || snap.stopping
        || matches!(snap.state(), MachineState::Run | MachineState::Jog | MachineState::Hold | MachineState::Home);
    if !moving || !matches!(ctrl.phase(), Phase::Ready | Phase::Connecting) {
        return;
    }
    ctrl.stop();
    let mut buf = [0u8; 1024];
    let end = now().saturating_add(budget_ms);
    let mut done = false;
    while now() < end {
        if step(ctrl, port, &mut buf, now()).is_err() {
            break;
        }
        if !ctrl.snapshot().stopping {
            // The banner is back: send the M5 that follows it (a port that fails now is closing
            // anyway, and the reset already turned the beam off).
            let _ = step(ctrl, port, &mut buf, now());
            done = true;
            break;
        }
    }
    if !done {
        // The last resort before the port closes; a port that fails here is gone already.
        let _ = port.write(&[HOLD, RESET]);
    }
}
