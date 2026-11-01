//! Picabia laser link (standalone: no other VectorCraft dependencies): talk to a laser cutter
//! running GRBL 1.1 over a serial port, written from GRBL's public documentation (the v1.1
//! Interface, Commands, Laser Mode and Configuration pages and the error and alarm code tables).
//!
//! # Parts
//! - [`proto`]: one line from the machine to a [`Response`] ([`parse_line`], status reports
//!   [`Status`]), a G-code line made safe to stream ([`clean_line`]), the realtime bytes.
//! - [`codes`]: error and alarm codes in plain English, and what to do about an alarm.
//! - [`Settings`]: the `$$` settings (`$32` laser mode, `$30` maximum power, travel, homing).
//! - [`Streamer`]: character counting, so the machine's 128-byte receive buffer never overflows.
//! - [`Controller`]: the protocol as a sans-IO state machine (handshake, status polling, one-off
//!   commands, jobs, pause, resume, the stop sequence). Bytes in with [`Controller::on_bytes`],
//!   bytes out from [`Controller::poll`], state out with [`Controller::snapshot`]. Time comes in
//!   as `now_ms`, so it runs the same natively, in the browser build and in tests.
//! - [`Transport`]: where the bytes go ([`pump`] moves them between a transport and a controller).
//! - `serial` (native only): the serial ports and a [`Transport`] over one.
//! - [`sim`]: a simulated GRBL machine behind the same [`Transport`], laser mode modelled, so tests
//!   can check that the beam is never on while travelling.
//!
//! # Safety
//! A job starts only when the controller is ready, the machine is Idle and `$32 = 1` (laser mode:
//! without it the beam stays on during rapid moves and dwells at corners). Stop is always allowed:
//! feed hold, then soft reset (which purges the machine's buffer and turns the laser off), then
//! `M5` once the machine is back. An error or alarm during a job stops it the same way.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod codes;
mod controller;
mod job;
pub mod proto;
#[cfg(not(target_arch = "wasm32"))]
pub mod serial;
mod settings;
pub mod sim;
mod streamer;
mod transport;

#[cfg(test)]
mod controller_tests;
#[cfg(test)]
mod proto_tests;
#[cfg(test)]
mod sim_tests;
#[cfg(test)]
mod streamer_tests;

pub use codes::{alarm_advice, alarm_text, error_text};
pub use controller::{Controller, HANDSHAKE_TIMEOUT_MS, LOG_LINES, MAX_JOB_LINES, STATUS_INTERVAL_MS};
pub use job::{AlarmInfo, Job, JobState, Phase, Refusal, Snapshot};
pub use proto::{HOLD, JOG_CANCEL, MAX_LINE, MachineState, RESET, RESUME, RX_BUFFER, Response, STATUS, Status, clean_line, parse_line};
pub use settings::Settings;
pub use streamer::Streamer;
pub use transport::{Transport, pump};
