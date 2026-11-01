//! The GRBL protocol as a sans-IO state machine. The owner (the engine's worker thread, or a test)
//! feeds it what the machine sent ([`Controller::on_bytes`]), writes what it returns from
//! [`Controller::poll`], and reads [`Controller::snapshot`]. Time is passed in (`now_ms`), so the
//! same code runs natively, in the browser build and in deterministic tests.

use std::collections::VecDeque;

use crate::job::{AlarmInfo, Job, JobState, Phase, Refusal, Snapshot};
use crate::proto::{HOLD, JOG_CANCEL, MAX_INCOMING, RESET, RESUME, RX_BUFFER, STATUS, cap_str};
use crate::{MachineState, Response, Settings, Status, Streamer, alarm_advice, alarm_text, clean_line, error_text, parse_line};

/// How long to wait for the banner before a soft reset, and again after it.
pub const HANDSHAKE_TIMEOUT_MS: u64 = 2500;
/// Status poll interval once ready (5 Hz, as the Interface page advises).
pub const STATUS_INTERVAL_MS: u64 = 200;
/// Lines kept in [`Snapshot::log`].
pub const LOG_LINES: usize = 200;
/// The most lines a job may have.
pub const MAX_JOB_LINES: usize = 4_000_000;

/// The stop sequence waits at least this long after the hold before the reset…
const HOLD_MIN_MS: u64 = 50;
/// …and resets at the latest after this, stopped or not (a reset while moving costs the position,
/// never safety).
const HOLD_MAX_MS: u64 = 500;
/// How long to wait for the banner after the stop's reset.
const RESET_TIMEOUT_MS: u64 = 2500;
/// Each log line is cut to this many bytes.
const LOG_LINE_BYTES: usize = 120;
/// One-off commands waiting to be sent.
const MAX_PENDING: usize = 64;
/// Appended to every job: laser off, then a dwell whose `ok` arrives only once every move is done.
const JOB_TAIL: [&str; 2] = ["M5", "G4P0.01"];

/// What a line in flight was, so its `ok`/`error` lands in the right place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Info,
    Settings,
    Command,
    /// Index into the job's lines.
    Job(usize),
    /// The `M5` after a reset.
    AfterReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handshake {
    WaitBanner { since: u64, reset_sent: bool },
    Querying,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// Feed hold sent at `since`; `seq` is the status count then (a newer report is needed).
    Hold { since: u64, seq: u64 },
    /// Soft reset sent at `since`; waiting for the banner.
    Reset { since: u64 },
}

/// The GRBL link's state machine. See the crate docs.
#[derive(Debug)]
pub struct Controller {
    phase: Phase,
    handshake: Handshake,
    now: u64,
    rx: Vec<u8>,
    rx_overlong: bool,
    streamer: Streamer,
    /// Kind and "must be alone in the buffer" of each line in flight, oldest first.
    in_flight: VecDeque<(Kind, bool)>,
    pending: VecDeque<(Kind, String)>,
    realtime: Vec<u8>,
    job: Option<Job>,
    job_lines: Vec<String>,
    /// 1-based line number in the caller's file of each job line.
    job_src: Vec<u32>,
    next_line: usize,
    /// Every job line answered when the status count was this; done on a later Idle report.
    all_acked_at: Option<u64>,
    stop: Option<Stop>,
    status: Option<Status>,
    status_seq: u64,
    wco: Option<[f64; 3]>,
    settings: Settings,
    version: Option<String>,
    build: Option<String>,
    alarm: Option<AlarmInfo>,
    last_error: Option<String>,
    log: VecDeque<String>,
    last_poll: Option<u64>,
}

impl Default for Controller {
    fn default() -> Self {
        Controller::new()
    }
}

impl Controller {
    pub fn new() -> Controller {
        Controller {
            phase: Phase::Closed,
            handshake: Handshake::Done,
            now: 0,
            rx: Vec::new(),
            rx_overlong: false,
            streamer: Streamer::new(RX_BUFFER),
            in_flight: VecDeque::new(),
            pending: VecDeque::new(),
            realtime: Vec::new(),
            job: None,
            job_lines: Vec::new(),
            job_src: Vec::new(),
            next_line: 0,
            all_acked_at: None,
            stop: None,
            status: None,
            status_seq: 0,
            wco: None,
            settings: Settings::default(),
            version: None,
            build: None,
            alarm: None,
            last_error: None,
            log: VecDeque::new(),
            last_poll: None,
        }
    }

    /// The port is open: forget the previous machine and wait for its banner.
    pub fn on_connected(&mut self, now_ms: u64) {
        let log = std::mem::take(&mut self.log);
        *self = Controller::new();
        self.log = log;
        self.now = now_ms;
        self.phase = Phase::Connecting;
        self.handshake = Handshake::WaitBanner { since: now_ms, reset_sent: false };
        self.push_log("-- connected".into());
    }

    /// The port went away: a running job fails at the line it reached.
    pub fn on_disconnect(&mut self) {
        if self.phase == Phase::Closed {
            return;
        }
        self.end_job_failed(self.reached_line(), "the connection to the machine was lost".into());
        self.phase = Phase::Lost;
        self.stop = None;
        self.streamer.clear();
        self.in_flight.clear();
        self.pending.clear();
        self.realtime.clear();
        self.push_log("-- disconnected".into());
    }

    /// Bytes the machine sent.
    pub fn on_bytes(&mut self, bytes: &[u8], now_ms: u64) {
        self.now = now_ms.max(self.now);
        if !matches!(self.phase, Phase::Connecting | Phase::Ready) {
            return;
        }
        for &b in bytes {
            match b {
                b'\n' => {
                    if self.rx_overlong {
                        self.push_log("< (an over-long line was ignored)".into());
                    } else {
                        let line = String::from_utf8_lossy(&self.rx).into_owned();
                        self.handle_line(&line);
                    }
                    self.rx.clear();
                    self.rx_overlong = false;
                }
                b'\r' => {}
                _ if self.rx.len() >= MAX_INCOMING => self.rx_overlong = true,
                _ => self.rx.push(b),
            }
        }
    }

    /// The bytes to write now: realtime bytes first, then the lines that fit the machine's buffer.
    pub fn poll(&mut self, now_ms: u64) -> Vec<u8> {
        self.now = now_ms.max(self.now);
        if !matches!(self.phase, Phase::Connecting | Phase::Ready) {
            self.realtime.clear();
            return Vec::new();
        }
        let mut out = std::mem::take(&mut self.realtime);
        self.step_handshake(&mut out);
        self.step_stop(&mut out);
        if self.phase == Phase::Ready && self.last_poll.is_none_or(|t| self.now.saturating_sub(t) >= STATUS_INTERVAL_MS) {
            out.push(STATUS);
            self.last_poll = Some(self.now);
        }
        self.stream(&mut out);
        out
    }

    /// A copy of the state for the panel.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.phase,
            version: self.version.clone(),
            build: self.build.clone(),
            settings: self.settings.clone(),
            status: self.status.clone(),
            job: self.job.clone(),
            alarm: self.alarm,
            last_error: self.last_error.clone(),
            log: self.log.clone(),
            stopping: self.stop.is_some(),
            rx_size: self.streamer.capacity(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The job's progress, if there was one since connecting.
    pub fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    /// Send one line (`$H`, `$X`, `$J=…`, `G10 L20 P1 X0 Y0`, …) when the buffer allows. Refused
    /// while a job runs or the stop sequence is under way.
    pub fn command(&mut self, line: &str) -> Result<(), Refusal> {
        if self.phase != Phase::Ready {
            return Err(Refusal::NotReady);
        }
        if self.stop.is_some() {
            return Err(Refusal::Stopping);
        }
        if self.job.as_ref().is_some_and(|j| j.state.is_active()) {
            return Err(Refusal::JobActive);
        }
        let clean = clean_line(line).map_err(|why| Refusal::BadLine { line: 1, why })?;
        if clean.is_empty() {
            return Err(Refusal::Empty);
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(Refusal::Busy);
        }
        self.pending.push_back((Kind::Command, clean));
        Ok(())
    }

    /// Start streaming a job. Refused unless ready, Idle, `$32 = 1` and no job is running; every
    /// line is cleaned first (comments and blanks dropped) and a bad line refuses the whole job.
    pub fn start_job<I, S>(&mut self, name: &str, lines: I) -> Result<(), Refusal>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.check_can_start()?;
        let mut clean = Vec::new();
        let mut src = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            if i >= MAX_JOB_LINES {
                return Err(Refusal::TooLong(i + 1));
            }
            let c = clean_line(line.as_ref()).map_err(|why| Refusal::BadLine { line: i + 1, why })?;
            if !c.is_empty() {
                clean.push(c);
                src.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
        }
        if clean.is_empty() {
            return Err(Refusal::Empty);
        }
        let count = clean.len();
        let last = src.last().copied().unwrap_or(0);
        for t in JOB_TAIL {
            clean.push(t.to_string());
            src.push(last);
        }
        self.job_lines = clean;
        self.job_src = src;
        self.next_line = 0;
        self.all_acked_at = None;
        self.job = Some(Job {
            name: cap_str(name, 200).to_string(),
            lines: count,
            sent: 0,
            acked: 0,
            state: JobState::Running,
            started_ms: self.now,
            ended_ms: None,
        });
        self.last_error = None;
        self.push_log(format!("-- job \"{}\" started: {count} lines", cap_str(name, 60)));
        Ok(())
    }

    fn check_can_start(&self) -> Result<(), Refusal> {
        if self.phase != Phase::Ready {
            return Err(Refusal::NotReady);
        }
        if self.stop.is_some() {
            return Err(Refusal::Stopping);
        }
        if self.job.as_ref().is_some_and(|j| j.state.is_active()) {
            return Err(Refusal::JobActive);
        }
        let state = self.status.as_ref().map(|s| s.state).unwrap_or_default();
        if state != MachineState::Idle {
            return Err(Refusal::NotIdle(state));
        }
        match self.settings.laser_mode() {
            Some(true) => Ok(()),
            Some(false) => Err(Refusal::LaserModeOff),
            None => Err(Refusal::LaserModeUnknown),
        }
    }

    /// Feed hold (`!`). Always sent; a running job becomes Paused.
    pub fn pause(&mut self) {
        if self.stop.is_some() {
            return;
        }
        self.realtime.push(HOLD);
        if let Some(job) = self.job.as_mut().filter(|j| j.state == JobState::Running) {
            job.state = JobState::Paused;
        }
        self.push_log("> ! (hold)".into());
    }

    /// Resume (`~`) after a hold; a paused job runs again. Never while stopping.
    pub fn resume(&mut self) {
        if self.stop.is_some() {
            return;
        }
        self.realtime.push(RESUME);
        if let Some(job) = self.job.as_mut().filter(|j| j.state == JobState::Paused) {
            job.state = JobState::Running;
        }
        self.push_log("> ~ (resume)".into());
    }

    /// Cancel a jog (`0x85`).
    pub fn jog_cancel(&mut self) {
        self.realtime.push(JOG_CANCEL);
        self.push_log("> jog cancel".into());
    }

    /// Stop: feed hold now, soft reset once the machine has stopped (or after a short wait), `M5`
    /// once it is back. Always allowed; a running job ends Stopped.
    pub fn stop(&mut self) {
        if !matches!(self.phase, Phase::Connecting | Phase::Ready) || self.stop.is_some() {
            return;
        }
        if let Some(job) = self.job.as_mut().filter(|j| j.state.is_active()) {
            job.state = JobState::Stopped;
            job.ended_ms = Some(self.now);
        }
        self.begin_stop();
        self.push_log("> ! (stop: hold, then reset)".into());
    }

    fn begin_stop(&mut self) {
        self.pending.clear();
        self.realtime.push(HOLD);
        self.realtime.push(STATUS);
        self.stop = Some(Stop::Hold { since: self.now, seq: self.status_seq });
    }

    fn step_handshake(&mut self, out: &mut Vec<u8>) {
        let Handshake::WaitBanner { since, reset_sent } = self.handshake else { return };
        if self.now.saturating_sub(since) < HANDSHAKE_TIMEOUT_MS {
            return;
        }
        if reset_sent {
            self.phase = Phase::Lost;
            self.last_error = Some("No answer from GRBL: is this the laser's port, at 115200 baud?".into());
            self.push_log("-- no banner after a reset: giving up".into());
        } else {
            out.push(RESET);
            self.handshake = Handshake::WaitBanner { since: self.now, reset_sent: true };
            self.push_log("> reset (no banner yet)".into());
        }
    }

    fn step_stop(&mut self, out: &mut Vec<u8>) {
        match self.stop {
            Some(Stop::Hold { since, seq }) => {
                let waited = self.now.saturating_sub(since);
                let stopped = self.status_seq > seq && self.status.as_ref().is_some_and(is_stopped);
                if (waited >= HOLD_MIN_MS && stopped) || waited >= HOLD_MAX_MS {
                    out.push(RESET);
                    self.purge();
                    self.stop = Some(Stop::Reset { since: self.now });
                    self.push_log("> reset".into());
                }
            }
            Some(Stop::Reset { since }) if self.now.saturating_sub(since) >= RESET_TIMEOUT_MS => {
                self.stop = None;
                self.last_error =
                    Some("The machine didn't come back after the reset: switch it off at the power switch before going near it.".into());
                self.push_log("-- no banner after the stop's reset".into());
            }
            _ => {}
        }
    }

    /// The machine dropped its receive buffer (reset): nothing is in flight any more.
    fn purge(&mut self) {
        self.streamer.clear();
        self.in_flight.clear();
        self.pending.clear();
    }

    fn stream(&mut self, out: &mut Vec<u8>) {
        if self.stop.is_some() || self.in_flight.iter().any(|(_, alone)| *alone) {
            return;
        }
        loop {
            let (kind, line) = if let Some((kind, line)) = self.pending.front() {
                (*kind, line.as_str())
            } else if self.job.as_ref().is_some_and(|j| j.state.is_active())
                && let Some(line) = self.job_lines.get(self.next_line)
            {
                (Kind::Job(self.next_line), line.as_str())
            } else {
                return;
            };
            let alone = needs_empty_buffer(line);
            if (alone && !self.streamer.is_empty()) || !self.streamer.push(line.len()) {
                return;
            }
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
            self.in_flight.push_back((kind, alone));
            if let Kind::Job(i) = kind {
                self.next_line = i + 1;
                if let Some(job) = self.job.as_mut() {
                    job.sent = self.next_line.min(job.lines);
                }
            } else {
                let text = format!("> {line}");
                self.pending.pop_front();
                self.push_log(text);
            }
            if alone {
                return;
            }
        }
    }

    fn handle_line(&mut self, line: &str) {
        match parse_line(line) {
            Response::Ok => self.on_answer(None),
            Response::Error(n) => self.on_answer(Some(n)),
            Response::Alarm(n) => self.on_alarm(n),
            Response::Status(s) => self.on_status(s),
            Response::Welcome { version } => self.on_welcome(version, line),
            Response::Setting(n, v) => {
                self.settings.set(n, &v);
                self.push_log(format!("< {line}"));
            }
            Response::Feedback(f) => {
                self.on_feedback(&f);
                self.push_log(format!("< {line}"));
            }
            Response::Message(_) => self.push_log(format!("< {line}")),
            Response::Other(s) => {
                if !s.is_empty() {
                    self.push_log(format!("< {line}"));
                }
            }
        }
    }

    fn on_feedback(&mut self, f: &str) {
        if let Some(v) = f.strip_prefix("VER:") {
            let build = v.split(':').next().unwrap_or("");
            self.build = Some(cap_str(build, 40).to_string());
        } else if let Some(opt) = f.strip_prefix("OPT:")
            && let Some(rx) = opt.split(',').nth(2).and_then(|n| n.trim().parse::<usize>().ok())
        {
            // Never count against more than the classic 128 bytes, whatever a build says.
            self.streamer.set_capacity(rx.clamp(16, RX_BUFFER));
        }
    }

    /// `ok` (`err` = None) or `error:N` answers the oldest line in flight.
    fn on_answer(&mut self, err: Option<u16>) {
        self.streamer.ack();
        let Some((kind, _)) = self.in_flight.pop_front() else {
            self.push_log(format!("< {} (nothing was waiting for it)", answer_text(err)));
            return;
        };
        match kind {
            Kind::Job(i) => self.on_job_answer(i, err),
            Kind::Settings => {
                self.handshake = Handshake::Done;
                self.phase = Phase::Ready;
                self.log_answer(err, "reading the settings ($$)");
                self.push_log("-- ready".into());
            }
            Kind::Info => self.log_answer(err, "reading the version ($I)"),
            Kind::Command => self.log_answer(err, "the command"),
            Kind::AfterReset => self.push_log(format!("< {} (M5 after the reset)", answer_text(err))),
        }
    }

    fn log_answer(&mut self, err: Option<u16>, what: &str) {
        self.push_log(format!("< {}", answer_text(err)));
        if let Some(n) = err {
            self.last_error = Some(format!("The machine refused {what}: error {n}, {}", error_text(n)));
        }
    }

    fn on_job_answer(&mut self, i: usize, err: Option<u16>) {
        if let Some(n) = err {
            let line = self.job_src.get(i).map_or(0, |&l| l as usize);
            self.push_log(format!("< error:{n} (job line {line})"));
            self.fail_job(line, format!("the machine refused line {line}: error {n}, {}", error_text(n)));
            return;
        }
        let total = self.job_lines.len();
        if let Some(job) = self.job.as_mut().filter(|j| j.state.is_active()) {
            job.acked = (i + 1).min(job.lines);
            if i + 1 >= total {
                self.all_acked_at = Some(self.status_seq);
                // Ask now rather than waiting for the next poll.
                self.realtime.push(STATUS);
            }
        }
    }

    fn on_alarm(&mut self, n: u16) {
        self.alarm = Some(AlarmInfo { code: n, text: alarm_text(n), advice: alarm_advice(n) });
        self.push_log(format!("< ALARM:{n}"));
        self.fail_job(self.reached_line(), format!("alarm {n}: {}", alarm_text(n)));
    }

    /// An error or alarm during a job: it fails and the stop sequence runs.
    fn fail_job(&mut self, line: usize, why: String) {
        if !self.job.as_ref().is_some_and(|j| j.state.is_active()) {
            return;
        }
        self.last_error = Some(format!("The job stopped: {why}"));
        self.end_job_failed(line, why);
        if self.stop.is_none() {
            self.begin_stop();
        }
    }

    fn end_job_failed(&mut self, line: usize, why: String) {
        if let Some(job) = self.job.as_mut().filter(|j| j.state.is_active()) {
            job.state = JobState::Failed { line, why };
            job.ended_ms = Some(self.now);
        }
    }

    /// The 1-based file line of the last line the machine accepted (0 before the first).
    fn reached_line(&self) -> usize {
        let acked = self.job.as_ref().map_or(0, |j| j.acked);
        acked.checked_sub(1).and_then(|i| self.job_src.get(i)).map_or(0, |&l| l as usize)
    }

    fn on_status(&mut self, mut s: Status) {
        if s.wco.is_some() {
            self.wco = s.wco;
        }
        if let Some(wco) = self.wco {
            match (s.mpos, s.wpos) {
                (Some(m), None) => s.wpos = Some([m[0] - wco[0], m[1] - wco[1], m[2] - wco[2]]),
                (None, Some(w)) => s.mpos = Some([w[0] + wco[0], w[1] + wco[1], w[2] + wco[2]]),
                _ => {}
            }
        }
        if s.state != MachineState::Alarm && s.state != MachineState::Unknown {
            self.alarm = None;
        }
        self.status_seq += 1;
        let idle = s.state == MachineState::Idle;
        self.status = Some(s);
        if idle && self.all_acked_at.is_some_and(|at| self.status_seq > at) {
            self.all_acked_at = None;
            if let Some(job) = self.job.as_mut().filter(|j| j.state.is_active()) {
                job.state = JobState::Done;
                job.ended_ms = Some(self.now);
                let text = format!("-- job \"{}\" done", cap_str(&job.name, 60));
                self.push_log(text);
            }
        }
    }

    fn on_welcome(&mut self, version: String, line: &str) {
        self.push_log(format!("< {line}"));
        self.version = Some(version);
        self.purge();
        self.status = None;
        self.all_acked_at = None;
        let after_ready = self.handshake == Handshake::Done;
        if self.stop.take().is_none() && after_ready {
            self.fail_job(self.reached_line(), "the machine reset itself (or lost power) during the job".into());
            // fail_job started a stop; the reset already happened.
            self.stop = None;
            self.realtime.retain(|&b| b != HOLD);
        }
        match self.handshake {
            Handshake::WaitBanner { .. } => {
                self.handshake = Handshake::Querying;
                self.pending.push_back((Kind::Info, "$I".into()));
                self.pending.push_back((Kind::Settings, "$$".into()));
            }
            Handshake::Querying => {
                self.pending.push_back((Kind::Info, "$I".into()));
                self.pending.push_back((Kind::Settings, "$$".into()));
            }
            // A reset leaves the laser off; M5 makes sure the parser agrees.
            Handshake::Done => self.pending.push_back((Kind::AfterReset, "M5".into())),
        }
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        let line = if line.len() > LOG_LINE_BYTES { cap_str(&line, LOG_LINE_BYTES).to_string() } else { line };
        self.log.push_back(line);
    }
}

/// Stopped moving: hold complete, idle, alarm, door, sleep.
fn is_stopped(s: &Status) -> bool {
    match s.state {
        MachineState::Hold => s.sub == Some(0),
        MachineState::Run | MachineState::Jog | MachineState::Home => false,
        MachineState::Door => matches!(s.sub, Some(0) | Some(1)),
        _ => true,
    }
}

/// Lines that change settings or the machine's state must go alone (the Interface page: don't
/// character-count EEPROM writes; `$` commands want an idle machine).
fn needs_empty_buffer(line: &str) -> bool {
    let up = line.to_ascii_uppercase();
    (up.starts_with('$') && !up.starts_with("$J=")) || up.contains("G10") || up.contains("G28.1") || up.contains("G30.1")
}

fn answer_text(err: Option<u16>) -> String {
    match err {
        None => "ok".into(),
        Some(n) => format!("error:{n}"),
    }
}
