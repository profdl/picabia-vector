//! A simulated GRBL 1.1 machine, written from GRBL's public documentation (Interface, Commands,
//! Laser Mode), for tests and for trying the laser panel without a laser. Deterministic: time only
//! moves in [`SimGrbl::advance`].
//!
//! Modelled: the receive buffer (overflowing it is recorded, never hidden), a 15-block planner
//! (lines are read only while it has room, so `ok`s are held back as on the real machine),
//! G0/G1/G2/G3 moves at feed × `speed_factor` (arcs as straight lines to their end point), G4
//! dwells (answered after the planner empties and the dwell ends), M3/M4/M5 (they wait for the
//! planner to empty), `S`, `F`, G10 L20, G20/G21, G90/G91, `$$`, `$I`, `$H`, `$X`, `$J=`, setting
//! writes, the realtime bytes (`?`, `!`, `~`, `0x18`, `0x85`), alarms and the welcome banner.
//!
//! The beam: with laser mode on (`$32=1`) it is on only during G1/G2/G3 moves with M3/M4 and
//! S > 0 (G0 is always beam-off; under M4 it is off whenever the machine stands still; in a hold
//! it is off). With laser mode off it is on whenever M3/M4 and S > 0, moving or not, G0 included.
//! [`SimStats`] records beam-on distance, beam-on distance on non-cutting moves and beam-on time
//! standing still, so tests can assert the laser never travels lit.

use std::collections::{BTreeMap, VecDeque};
use std::io;

use crate::{MachineState, Transport};

mod gcode;

/// Planner blocks (`[OPT:…,15,…]` on the classic builds).
const PLANNER_BLOCKS: usize = 15;
/// Bytes the simulated machine queues for reading before it drops output.
const MAX_OUTPUT: usize = 1 << 20;
/// Time to decelerate into a hold (ms).
const HOLD_DECEL_MS: f64 = 30.0;
/// Homing takes this long (ms, before `speed_factor`).
const HOMING_MS: f64 = 1500.0;
/// GRBL's line buffer (longer lines get `error:11`).
const LINE_MAX: usize = 80;

/// How the simulated machine is set up.
#[derive(Clone, Debug, PartialEq)]
pub struct SimOptions {
    /// `$32`.
    pub laser_mode: bool,
    /// `$30`.
    pub s_max: f64,
    /// `$130`–`$132` (mm).
    pub max_travel: [f64; 3],
    /// Receive buffer (bytes).
    pub rx_size: usize,
    /// Moves run this many times faster than their feed.
    pub speed_factor: f64,
    /// Raise `ALARM:1` (hard limit) when this G-code line (1-based, `$` lines not counted) is read.
    pub alarm_at_line: Option<usize>,
    /// Answer `error:20` to this G-code line (1-based, `$` lines not counted).
    pub error_at_line: Option<usize>,
    /// `$22`: homing enabled.
    pub homing: bool,
    /// Start in alarm (as a machine with homing enabled does at power-up).
    pub start_in_alarm: bool,
    /// Print the banner when the port opens (a board that resets on DTR); otherwise the machine
    /// says nothing until it is reset.
    pub banner_on_open: bool,
    /// G0 and the default rapid rate (`$110`/`$111`, mm/min).
    pub rapid_mm_min: f64,
}

impl Default for SimOptions {
    fn default() -> Self {
        SimOptions {
            laser_mode: true,
            s_max: 1000.0,
            max_travel: [584.2, 1219.2, 50.0],
            rx_size: crate::RX_BUFFER,
            speed_factor: 1.0,
            alarm_at_line: None,
            error_at_line: None,
            homing: false,
            start_in_alarm: false,
            banner_on_open: true,
            rapid_mm_min: 5000.0,
        }
    }
}

/// What happened, for tests to assert.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimStats {
    /// Bytes that arrived with the receive buffer full (must stay 0).
    pub overflows: usize,
    /// The fullest the receive buffer got.
    pub max_rx_used: usize,
    /// Distance moved with the beam on (mm).
    pub burn_mm: f64,
    /// Distance moved with the beam on during G0 or jog moves (mm; must stay 0 in laser mode).
    pub beam_on_travel_mm: f64,
    /// Time with the beam on while standing still (ms).
    pub beam_on_still_ms: f64,
    /// Total distance moved (mm).
    pub moved_mm: f64,
    /// G-code lines read (not `$` lines).
    pub gcode_lines: usize,
    /// Lines answered `ok`.
    pub oks: usize,
    /// Lines answered `error:N`.
    pub errors: usize,
    /// Soft resets received.
    pub resets: usize,
    /// Status reports sent.
    pub reports: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spindle {
    Off,
    Cw,
    Ccw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveKind {
    Rapid,
    Feed,
    Jog,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Block {
    kind: MoveKind,
    target: [f64; 3],
    /// mm/min.
    feed: f64,
    spindle: Spindle,
    s: f64,
    /// The modal motion was G1/G2/G3 (a jog inherits the beam from it).
    cutting_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Modal {
    motion: u8,
    absolute: bool,
    inches: bool,
    feed: f64,
    spindle: Spindle,
    s: f64,
}

impl Default for Modal {
    fn default() -> Self {
        Modal { motion: 0, absolute: true, inches: false, feed: 0.0, spindle: Spindle::Off, s: 0.0 }
    }
}

/// The simulated machine.
#[derive(Clone, Debug)]
pub struct SimGrbl {
    opt: SimOptions,
    t: f64,
    rx: VecDeque<u8>,
    out: VecDeque<u8>,
    state: MachineState,
    /// Hold: decelerating until this time; `None` once complete.
    hold_until: Option<f64>,
    alarm: Option<u16>,
    /// After a hard or soft limit alarm GRBL may ignore `?` until reset.
    critical: bool,
    planner: VecDeque<Block>,
    pos: [f64; 3],
    wco: [f64; 3],
    modal: Modal,
    /// A G4 dwell running until this time; its `ok` is sent then.
    dwell_until: Option<f64>,
    home_until: Option<f64>,
    settings: BTreeMap<u16, f64>,
    stats: SimStats,
}

impl SimGrbl {
    pub fn new(opt: SimOptions) -> SimGrbl {
        let mut settings = BTreeMap::new();
        let rapid = opt.rapid_mm_min;
        for (n, v) in [
            (0, 10.0),
            (1, 25.0),
            (10, 1.0),
            (20, 0.0),
            (21, 0.0),
            (22, if opt.homing { 1.0 } else { 0.0 }),
            (27, 1.0),
            (30, opt.s_max),
            (31, 0.0),
            (32, if opt.laser_mode { 1.0 } else { 0.0 }),
            (100, 40.0),
            (101, 40.0),
            (102, 188.976),
            (110, rapid),
            (111, rapid),
            (112, 500.0),
            (120, 500.0),
            (121, 500.0),
            (122, 50.0),
            (130, opt.max_travel[0]),
            (131, opt.max_travel[1]),
            (132, opt.max_travel[2]),
        ] {
            settings.insert(n, v);
        }
        let mut sim = SimGrbl {
            state: if opt.start_in_alarm { MachineState::Alarm } else { MachineState::Idle },
            opt,
            t: 0.0,
            rx: VecDeque::new(),
            out: VecDeque::new(),
            hold_until: None,
            alarm: None,
            critical: false,
            planner: VecDeque::new(),
            pos: [0.0; 3],
            wco: [0.0; 3],
            modal: Modal::default(),
            dwell_until: None,
            home_until: None,
            settings,
            stats: SimStats::default(),
        };
        if sim.opt.banner_on_open {
            sim.banner();
        }
        sim
    }

    pub fn options(&self) -> &SimOptions {
        &self.opt
    }

    pub fn stats(&self) -> &SimStats {
        &self.stats
    }

    pub fn state(&self) -> MachineState {
        self.state
    }

    /// The current time (ms).
    pub fn now_ms(&self) -> u64 {
        self.t.round() as u64
    }

    /// Machine position (mm).
    pub fn mpos(&self) -> [f64; 3] {
        self.pos
    }

    /// Work position (mm).
    pub fn wpos(&self) -> [f64; 3] {
        [self.pos[0] - self.wco[0], self.pos[1] - self.wco[1], self.pos[2] - self.wco[2]]
    }

    /// Bytes waiting in the receive buffer.
    pub fn rx_used(&self) -> usize {
        self.rx.len()
    }

    /// Blocks waiting in the planner (the one executing included).
    pub fn planner_len(&self) -> usize {
        self.planner.len()
    }

    /// M3 or M4 is on (the parser's state, not the beam).
    pub fn spindle_on(&self) -> bool {
        self.modal.spindle != Spindle::Off
    }

    /// The beam is on right now.
    pub fn beam_on(&self) -> bool {
        match self.planner.front() {
            Some(b) if self.moving() => self.block_beam(b),
            _ => self.still_beam(),
        }
    }

    /// Raise an alarm now (a limit switch, a probe failure…): motion stops, the planner is flushed.
    pub fn raise_alarm(&mut self, code: u16) {
        self.planner.clear();
        self.dwell_until = None;
        self.home_until = None;
        self.hold_until = None;
        self.state = MachineState::Alarm;
        self.alarm = Some(code);
        self.critical = code == 1 || code == 2;
        self.emit(&format!("ALARM:{code}"));
        if self.critical {
            self.emit("[MSG:Reset to continue]");
        }
    }

    /// Bytes from the sender. Realtime bytes act at once; others go into the receive buffer
    /// (recorded as an overflow when it is full) and are read as the planner allows.
    pub fn receive(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'?' => self.report(),
                b'!' => self.feed_hold(),
                b'~' => self.cycle_start(),
                0x18 => self.soft_reset(),
                0x85 => self.jog_cancel(),
                0x80..=0xff => {}
                _ => {
                    if self.rx.len() >= self.opt.rx_size {
                        self.stats.overflows += 1;
                    } else {
                        self.rx.push_back(b);
                        self.stats.max_rx_used = self.stats.max_rx_used.max(self.rx.len());
                    }
                }
            }
        }
        self.read_lines();
    }

    /// Take up to `buf.len()` bytes of what the machine sent.
    pub fn read_out(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        for slot in buf.iter_mut() {
            let Some(b) = self.out.pop_front() else { break };
            *slot = b;
            n += 1;
        }
        n
    }

    /// Let `ms` milliseconds pass: moves run, dwells end, waiting lines are read.
    pub fn advance(&mut self, ms: u64) {
        let end = self.t + ms as f64;
        let mut guard = 0u32;
        loop {
            self.read_lines();
            let left = end - self.t;
            if left <= 0.0 || guard > 1_000_000 {
                break;
            }
            guard += 1;
            let used = self.run_for(left);
            if used <= 0.0 {
                if self.still_beam() {
                    self.stats.beam_on_still_ms += left;
                }
                self.t = end;
            } else {
                self.t += used;
            }
        }
        self.t = self.t.max(end);
    }

    fn moving(&self) -> bool {
        matches!(self.state, MachineState::Run | MachineState::Jog) && self.dwell_until.is_none()
    }

    fn block_beam(&self, b: &Block) -> bool {
        let lit = b.spindle != Spindle::Off && b.s > 0.0;
        if !self.opt.laser_mode {
            return lit;
        }
        match b.kind {
            MoveKind::Feed => lit,
            MoveKind::Jog => lit && b.cutting_mode,
            MoveKind::Rapid => false,
        }
    }

    /// The beam while standing still.
    fn still_beam(&self) -> bool {
        let lit = self.modal.spindle != Spindle::Off && self.modal.s > 0.0;
        if !self.opt.laser_mode {
            return lit && self.state != MachineState::Alarm;
        }
        // Laser mode: M4 is off when not moving; M3 stays on in a cutting mode, except in a hold
        // or an alarm.
        lit && self.modal.spindle == Spindle::Cw
            && (1..=3).contains(&self.modal.motion)
            && matches!(self.state, MachineState::Idle | MachineState::Run)
    }

    /// Run for at most `dt` ms; the time used (0 when there is nothing to do).
    fn run_for(&mut self, dt: f64) -> f64 {
        if let Some(until) = self.home_until {
            let used = (until - self.t).clamp(0.0, dt);
            if self.t + used >= until {
                self.finish_homing();
            }
            return used.max(f64::MIN_POSITIVE);
        }
        if self.state == MachineState::Hold {
            if let Some(until) = self.hold_until {
                let used = (until - self.t).clamp(0.0, dt);
                if self.t + used >= until {
                    self.hold_until = None;
                }
                return used.max(f64::MIN_POSITIVE);
            }
            return 0.0;
        }
        if let Some(until) = self.dwell_until {
            let used = (until - self.t).clamp(0.0, dt);
            if self.t + used >= until {
                self.dwell_until = None;
                self.ok();
            }
            if self.still_beam() {
                self.stats.beam_on_still_ms += used;
            }
            return used.max(f64::MIN_POSITIVE);
        }
        if !matches!(self.state, MachineState::Run | MachineState::Jog) {
            return 0.0;
        }
        let Some(block) = self.planner.front().copied() else {
            self.state = MachineState::Idle;
            return 0.0;
        };
        self.run_block(&block, dt)
    }

    fn run_block(&mut self, b: &Block, dt: f64) -> f64 {
        let d = [b.target[0] - self.pos[0], b.target[1] - self.pos[1], b.target[2] - self.pos[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let speed = (b.feed.max(1.0) * self.opt.speed_factor.max(1e-6)) / 60_000.0; // mm per ms
        let need = len / speed;
        let lit = self.block_beam(b);
        let (used, step) = if need <= dt { (need, len) } else { (dt, speed * dt) };
        if need <= dt {
            self.pos = b.target;
            self.planner.pop_front();
            if self.planner.is_empty() {
                self.state = MachineState::Idle;
            }
        } else if len > 0.0 {
            let k = step / len;
            for (p, di) in self.pos.iter_mut().zip(d) {
                *p += di * k;
            }
        }
        self.stats.moved_mm += step;
        if lit {
            self.stats.burn_mm += step;
            if b.kind != MoveKind::Feed {
                self.stats.beam_on_travel_mm += step;
            }
        }
        used.max(f64::MIN_POSITIVE)
    }

    fn finish_homing(&mut self) {
        self.home_until = None;
        self.pos = [0.0; 3];
        self.state = MachineState::Idle;
        self.alarm = None;
        self.ok();
    }

    /// Read complete lines from the receive buffer while the machine can take them.
    fn read_lines(&mut self) {
        loop {
            if self.home_until.is_some() || self.dwell_until.is_some() || self.planner.len() >= PLANNER_BLOCKS {
                return;
            }
            let Some(nl) = self.rx.iter().position(|&b| b == b'\n') else { return };
            let bytes: Vec<u8> = self.rx.iter().take(nl).copied().collect();
            let line = String::from_utf8_lossy(&bytes).trim().to_ascii_uppercase();
            if self.needs_sync(&line) && (!self.planner.is_empty() || self.state == MachineState::Hold) {
                return;
            }
            self.rx.drain(..=nl);
            if line.len() > LINE_MAX {
                self.error(11);
            } else {
                self.line(&line);
            }
        }
    }

    fn feed_hold(&mut self) {
        match self.state {
            MachineState::Run => {
                self.state = MachineState::Hold;
                self.hold_until = Some(self.t + HOLD_DECEL_MS);
            }
            MachineState::Jog => self.jog_cancel(),
            _ => {}
        }
    }

    fn cycle_start(&mut self) {
        if self.state == MachineState::Hold {
            self.hold_until = None;
            self.state = if self.planner.is_empty() { MachineState::Idle } else { MachineState::Run };
        }
    }

    fn jog_cancel(&mut self) {
        if self.state == MachineState::Jog {
            self.planner.retain(|b| b.kind != MoveKind::Jog);
            self.state = MachineState::Idle;
        }
    }

    fn soft_reset(&mut self) {
        self.stats.resets += 1;
        let moving = matches!(self.state, MachineState::Run | MachineState::Jog | MachineState::Home)
            || (self.state == MachineState::Hold && self.hold_until.is_some());
        self.rx.clear();
        self.planner.clear();
        self.dwell_until = None;
        self.home_until = None;
        self.hold_until = None;
        self.modal = Modal::default();
        self.critical = false;
        if moving {
            self.state = MachineState::Alarm;
            self.alarm = Some(3);
            self.emit("ALARM:3");
        } else if self.state != MachineState::Alarm {
            self.state = MachineState::Idle;
        }
        self.banner();
    }

    fn banner(&mut self) {
        self.emit("");
        self.emit("Grbl 1.1h ['$' for help]");
        if self.state == MachineState::Alarm {
            self.emit("[MSG:'$H'|'$X' to unlock]");
        }
    }

    fn report(&mut self) {
        if self.critical || self.home_until.is_some() {
            return;
        }
        self.stats.reports += 1;
        let state = match (self.state, self.hold_until) {
            (MachineState::Hold, Some(_)) => "Hold:1".to_string(),
            (MachineState::Hold, None) => "Hold:0".to_string(),
            (s, _) => s.as_str().to_string(),
        };
        let p = self.pos;
        let (feed, s) = match self.planner.front() {
            Some(b) if self.moving() => (b.feed, if b.spindle != Spindle::Off { b.s } else { 0.0 }),
            _ => (0.0, if self.modal.spindle != Spindle::Off { self.modal.s } else { 0.0 }),
        };
        let acc = match self.modal.spindle {
            Spindle::Off => "",
            Spindle::Cw => "|A:S",
            Spindle::Ccw => "|A:C",
        };
        let line = format!(
            "<{state}|MPos:{},{},{}|Bf:{},{}|FS:{},{}|WCO:{},{},{}|Ov:100,100,100{acc}>",
            num(p[0]),
            num(p[1]),
            num(p[2]),
            PLANNER_BLOCKS.saturating_sub(self.planner.len()),
            self.opt.rx_size.saturating_sub(self.rx.len()),
            num(feed),
            num(s),
            num(self.wco[0]),
            num(self.wco[1]),
            num(self.wco[2]),
        );
        self.emit(&line);
    }

    fn ok(&mut self) {
        self.stats.oks += 1;
        self.emit("ok");
    }

    fn error(&mut self, code: u16) {
        self.stats.errors += 1;
        self.emit(&format!("error:{code}"));
    }

    fn emit(&mut self, line: &str) {
        if self.out.len() + line.len() + 2 > MAX_OUTPUT {
            return;
        }
        self.out.extend(line.bytes());
        self.out.extend(b"\r\n");
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" { "0.000".into() } else { s }
}

/// [`SimGrbl`] behind [`Transport`], advanced to the time a clock gives on every read and write.
pub struct SimTransport {
    sim: SimGrbl,
    clock: Box<dyn FnMut() -> u64 + Send>,
}

impl SimTransport {
    pub fn new(sim: SimGrbl, clock: Box<dyn FnMut() -> u64 + Send>) -> SimTransport {
        SimTransport { sim, clock }
    }

    pub fn sim(&self) -> &SimGrbl {
        &self.sim
    }

    pub fn sim_mut(&mut self) -> &mut SimGrbl {
        &mut self.sim
    }

    fn sync(&mut self) {
        let now = (self.clock)();
        let at = self.sim.now_ms();
        if now > at {
            self.sim.advance(now - at);
        }
    }
}

impl Transport for SimTransport {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.sync();
        self.sim.receive(data);
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.sync();
        Ok(self.sim.read_out(buf))
    }

    fn name(&self) -> &str {
        "Simulated GRBL"
    }
}
