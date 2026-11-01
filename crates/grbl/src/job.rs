//! What the [`crate::Controller`] reports: the snapshot, the job, refusals.

use std::collections::VecDeque;
use std::fmt;

use crate::{MachineState, Settings, Status};

/// Where the link is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Not connected yet ([`crate::Controller::new`] before `on_connected`).
    #[default]
    Closed,
    /// Connected; waiting for the banner, then reading `$I` and `$$`.
    Connecting,
    /// The machine answered and its settings are read: commands and jobs may run.
    Ready,
    /// The port went away, or the machine never answered.
    Lost,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Closed => "closed",
            Phase::Connecting => "connecting",
            Phase::Ready => "ready",
            Phase::Lost => "lost",
        }
    }
}

/// How a job is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    /// Feed hold (`!`): the machine decelerated and waits for resume.
    Paused,
    /// Every line answered `ok` and the machine is Idle again.
    Done,
    /// Stopped by the person (hold, reset, M5).
    Stopped,
    /// Ended by the machine: `line` is the 1-based line of the file it reached or that failed.
    Failed {
        line: usize,
        why: String,
    },
}

impl JobState {
    /// Running or paused: the job still owns the machine.
    pub fn is_active(&self) -> bool {
        matches!(self, JobState::Running | JobState::Paused)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Done => "done",
            JobState::Stopped => "stopped",
            JobState::Failed { .. } => "failed",
        }
    }
}

/// A job's progress (the lines themselves stay in the controller).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    /// Lines to send (after comments and blank lines are removed).
    pub lines: usize,
    /// Lines sent to the machine.
    pub sent: usize,
    /// Lines the machine answered `ok`.
    pub acked: usize,
    pub state: JobState,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// An alarm the machine raised, in plain words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlarmInfo {
    pub code: u16,
    pub text: &'static str,
    pub advice: &'static str,
}

/// Everything the panel shows, copied out of the controller.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub phase: Phase,
    /// From the banner, e.g. `1.1h`.
    pub version: Option<String>,
    /// From `$I`'s `[VER:…]`, e.g. `1.1h.20190825`.
    pub build: Option<String>,
    pub settings: Settings,
    /// The latest status report, with both positions when the work offset is known.
    pub status: Option<Status>,
    pub job: Option<Job>,
    pub alarm: Option<AlarmInfo>,
    pub last_error: Option<String>,
    /// The last [`crate::LOG_LINES`] lines in (`< `) and out (`> `); status polls and the job's
    /// own lines and `ok`s are left out.
    pub log: VecDeque<String>,
    /// The stop sequence (hold, reset, M5) is under way.
    pub stopping: bool,
    /// The receive buffer counted against (128, or less when `$I` reports less).
    pub rx_size: usize,
}

impl Snapshot {
    /// The machine state from the latest status report.
    pub fn state(&self) -> MachineState {
        self.status.as_ref().map(|s| s.state).unwrap_or_default()
    }
}

/// Why a command or a job was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not connected, or still reading the machine's settings.
    NotReady,
    /// The stop sequence is under way.
    Stopping,
    /// A job is running or paused.
    JobActive,
    /// The machine isn't Idle.
    NotIdle(MachineState),
    /// `$32 = 0`.
    LaserModeOff,
    /// `$32` wasn't reported.
    LaserModeUnknown,
    /// Nothing to send.
    Empty,
    /// More lines than [`crate::MAX_JOB_LINES`].
    TooLong(usize),
    /// A line can't be sent (1-based line number, reason).
    BadLine { line: usize, why: String },
    /// Too many commands waiting.
    Busy,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotReady => write!(f, "The laser isn't connected and ready yet."),
            Refusal::Stopping => write!(f, "The laser is stopping; wait until it is back."),
            Refusal::JobActive => write!(f, "A job is running: pause or stop it first."),
            Refusal::NotIdle(s) => write!(f, "The machine must be Idle to start (it is {}).", s.as_str()),
            Refusal::LaserModeOff => {
                write!(f, "Laser mode is off ($32=0): the beam would stay on during moves between cuts. Send $32=1 to turn it on, then start again.")
            }
            Refusal::LaserModeUnknown => write!(f, "The machine didn't report laser mode ($32), so a job can't start safely."),
            Refusal::Empty => write!(f, "There is nothing to send."),
            Refusal::TooLong(n) => write!(f, "The job has {n} lines, more than the {} a job may have.", crate::MAX_JOB_LINES),
            Refusal::BadLine { line, why } => write!(f, "Line {line} can't be sent: it {why}."),
            Refusal::Busy => write!(f, "Too many commands are waiting; try again in a moment."),
        }
    }
}

impl std::error::Error for Refusal {}
