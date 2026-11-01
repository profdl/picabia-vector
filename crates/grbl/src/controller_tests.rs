use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::sim::{SimGrbl, SimOptions, SimTransport};
use crate::{Controller, JobState, MachineState, Phase, Refusal, Transport, pump};

pub(crate) struct Rig {
    pub ctrl: Controller,
    pub link: SimTransport,
    clock: Arc<AtomicU64>,
}

impl Rig {
    pub fn new(opt: SimOptions) -> Rig {
        let clock = Arc::new(AtomicU64::new(0));
        let c = clock.clone();
        let link = SimTransport::new(SimGrbl::new(opt), Box::new(move || c.load(Ordering::SeqCst)));
        let mut ctrl = Controller::new();
        ctrl.on_connected(0);
        Rig { ctrl, link, clock }
    }

    /// Connected, settings read and an Idle (or Alarm) status in.
    pub fn ready(opt: SimOptions) -> Rig {
        let mut rig = Rig::new(opt);
        assert!(rig.run_until(10_000, |r| r.ctrl.phase() == Phase::Ready && r.ctrl.snapshot().status.is_some()));
        rig
    }

    pub fn now(&self) -> u64 {
        self.clock.load(Ordering::SeqCst)
    }

    pub fn step(&mut self, ms: u64) {
        let now = self.clock.fetch_add(ms, Ordering::SeqCst) + ms;
        pump(&mut self.ctrl, &mut self.link, now).unwrap();
        let st = self.link.sim().stats();
        assert_eq!(st.overflows, 0, "receive buffer overflowed");
        assert!(st.max_rx_used <= 128);
    }

    pub fn run_until(&mut self, max_ms: u64, done: impl Fn(&Rig) -> bool) -> bool {
        let end = self.now() + max_ms;
        while self.now() < end {
            self.step(1);
            if done(self) {
                return true;
            }
        }
        false
    }

    pub fn sim(&self) -> &SimGrbl {
        self.link.sim()
    }

    fn job_state(&self) -> Option<JobState> {
        self.ctrl.job().map(|j| j.state.clone())
    }

    fn log_has(&self, needle: &str) -> bool {
        self.ctrl.snapshot().log.iter().any(|l| l.contains(needle))
    }
}

/// A square cut: travel G0, beam on only under G1.
pub(crate) fn square_job(size: f64) -> Vec<String> {
    [
        "; Picabia test".to_string(),
        "G21".into(),
        "G90".into(),
        "M5".into(),
        "G0 X10 Y10".into(),
        "M4 S500".into(),
        format!("G1 X{} Y10 F1200", 10.0 + size),
        format!("G1 X{} Y{}", 10.0 + size, 10.0 + size),
        format!("G1 X10 Y{}", 10.0 + size),
        "G1 X10 Y10".into(),
        "M5".into(),
        "G0 X0 Y0".into(),
        "M5".into(),
    ]
    .to_vec()
}

/// Many short cuts with travels between them.
pub(crate) fn long_job(cuts: usize) -> Vec<String> {
    let mut out = vec!["G21".to_string(), "G90".into(), "M5".into(), "M4 S800".into()];
    for i in 0..cuts {
        let x = (i % 100) as f64 * 2.0;
        let y = (i / 100 % 100) as f64 * 2.0;
        out.push(format!("G0 X{x:.3} Y{y:.3}"));
        out.push(format!("G1 X{:.3} Y{y:.3} F1500", x + 1.0));
        out.push(format!("G1 X{:.3} Y{:.3}", x + 1.0, y + 1.0));
    }
    out.push("M5".into());
    out.push("G0 X0 Y0".into());
    out
}

#[test]
fn connects_and_reads_the_machine() {
    let rig = Rig::ready(SimOptions::default());
    let s = rig.ctrl.snapshot();
    assert_eq!(s.phase, Phase::Ready);
    assert_eq!(s.version.as_deref(), Some("1.1h"));
    assert_eq!(s.build.as_deref(), Some("1.1h.SIM"));
    assert_eq!(s.settings.laser_mode(), Some(true));
    assert_eq!(s.settings.s_max(), Some(1000.0));
    assert_eq!(s.settings.max_travel(), Some([584.2, 1219.2, 50.0]));
    assert_eq!(s.rx_size, 128);
    assert_eq!(s.state(), MachineState::Idle);
    assert!(s.status.as_ref().unwrap().wpos.is_some(), "work position derived from MPos and WCO");
    assert!(s.log.iter().any(|l| l == "> $$"));
}

#[test]
fn resets_when_no_banner_then_connects() {
    let mut rig = Rig::new(SimOptions { banner_on_open: false, ..SimOptions::default() });
    assert!(rig.run_until(10_000, |r| r.ctrl.phase() == Phase::Ready));
    assert!(rig.now() >= 2500);
    assert_eq!(rig.sim().stats().resets, 1);
    assert!(rig.log_has("reset (no banner yet)"));
}

struct Silent;
impl Transport for Silent {
    fn write(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
    fn name(&self) -> &str {
        "silent"
    }
}

#[test]
fn gives_up_on_a_silent_port() {
    let mut ctrl = Controller::new();
    ctrl.on_connected(0);
    let mut t = Silent;
    for now in (0..6000).step_by(10) {
        pump(&mut ctrl, &mut t, now).unwrap();
    }
    let s = ctrl.snapshot();
    assert_eq!(s.phase, Phase::Lost);
    assert!(s.last_error.unwrap().contains("No answer"));
    assert_eq!(ctrl.start_job("x", ["G0X1"]), Err(Refusal::NotReady));
}

#[test]
fn job_runs_to_done_beam_off_on_travel() {
    let mut rig = Rig::ready(SimOptions { speed_factor: 10.0, ..SimOptions::default() });
    rig.ctrl.start_job("square", square_job(40.0)).unwrap();
    let job = rig.ctrl.job().unwrap();
    assert_eq!(job.lines, 12, "the comment line is dropped");
    assert!(rig.run_until(60_000, |r| r.job_state() == Some(JobState::Done)));
    let job = rig.ctrl.job().unwrap();
    assert_eq!((job.sent, job.acked), (12, 12));
    assert!(job.ended_ms.is_some());
    let st = rig.sim().stats();
    assert_eq!(st.beam_on_travel_mm, 0.0);
    assert!((st.burn_mm - 160.0).abs() < 1e-6, "{st:?}");
    assert_eq!(st.beam_on_still_ms, 0.0);
    assert!(!rig.sim().spindle_on());
    assert_eq!(rig.sim().mpos(), [0.0, 0.0, 0.0]);
    assert_eq!(rig.sim().state(), MachineState::Idle);
    assert!(rig.log_has("done"));
    // Another job may start now.
    rig.ctrl.start_job("again", square_job(5.0)).unwrap();
}

#[test]
fn pause_and_resume() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.start_job("square", square_job(100.0)).unwrap();
    assert!(rig.run_until(10_000, |r| r.sim().stats().burn_mm > 5.0));
    rig.ctrl.pause();
    assert_eq!(rig.job_state(), Some(JobState::Paused));
    assert!(rig.run_until(2000, |r| r.ctrl.snapshot().state() == MachineState::Hold && r.sim().state() == MachineState::Hold));
    rig.step(200);
    let at = rig.sim().mpos();
    let burn = rig.sim().stats().burn_mm;
    for _ in 0..50 {
        rig.step(10);
        assert!(!rig.sim().beam_on());
    }
    assert_eq!(rig.sim().mpos(), at, "nothing moves while paused");
    assert_eq!(rig.sim().stats().burn_mm, burn);
    assert_eq!(rig.ctrl.start_job("other", square_job(1.0)), Err(Refusal::JobActive));
    assert_eq!(rig.ctrl.command("$H"), Err(Refusal::JobActive));
    rig.ctrl.resume();
    assert_eq!(rig.job_state(), Some(JobState::Running));
    assert!(rig.run_until(120_000, |r| r.job_state() == Some(JobState::Done)));
    assert_eq!(rig.sim().stats().beam_on_travel_mm, 0.0);
}

#[test]
fn stop_mid_job_holds_resets_and_turns_the_laser_off() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.start_job("long", long_job(2000)).unwrap();
    assert!(rig.run_until(10_000, |r| r.sim().stats().burn_mm > 3.0 && r.sim().state() == MachineState::Run));
    rig.ctrl.stop();
    assert_eq!(rig.job_state(), Some(JobState::Stopped));
    assert!(rig.ctrl.snapshot().stopping);
    assert_eq!(rig.ctrl.start_job("x", square_job(1.0)), Err(Refusal::Stopping));
    assert!(rig.run_until(5000, |r| !r.ctrl.snapshot().stopping));
    assert_eq!(rig.sim().stats().resets, 1);
    // The reset came once the machine had stopped: no alarm 3, the position is kept.
    assert_ne!(rig.sim().state(), MachineState::Alarm);
    assert!(rig.run_until(1000, |r| r.log_has("> M5") && r.log_has("(M5 after the reset)")));
    let at = rig.sim().mpos();
    for _ in 0..100 {
        rig.step(10);
        assert!(!rig.sim().beam_on());
    }
    assert_eq!(rig.sim().mpos(), at, "buffer purged: nothing more runs");
    assert_eq!(rig.sim().planner_len(), 0);
    assert!(!rig.sim().spindle_on());
    let job = rig.ctrl.job().unwrap();
    assert!(job.acked < job.lines);
    // Ready for the next job.
    assert!(rig.run_until(1000, |r| r.ctrl.snapshot().state() == MachineState::Idle));
    rig.ctrl.start_job("next", square_job(5.0)).unwrap();
}

#[test]
fn stop_when_idle_is_harmless_and_always_allowed() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.stop();
    assert!(rig.run_until(3000, |r| !r.ctrl.snapshot().stopping));
    assert_eq!(rig.sim().state(), MachineState::Idle);
    assert!(rig.ctrl.job().is_none());
}

#[test]
fn alarm_mid_job_fails_and_stops() {
    let mut rig = Rig::ready(SimOptions { alarm_at_line: Some(40), ..SimOptions::default() });
    rig.ctrl.start_job("long", long_job(200)).unwrap();
    assert!(rig.run_until(30_000, |r| matches!(r.job_state(), Some(JobState::Failed { .. }))));
    let Some(JobState::Failed { line, why }) = rig.job_state() else { panic!() };
    assert!(why.contains("alarm 1"), "{why}");
    assert!(line > 0 && line < 50, "{line}");
    let s = rig.ctrl.snapshot();
    assert_eq!(s.alarm.unwrap().code, 1);
    assert!(s.alarm.unwrap().advice.contains("home"));
    assert!(s.last_error.unwrap().contains("alarm"));
    // A critical alarm answers no status: the stop resets after its wait anyway.
    assert!(rig.run_until(5000, |r| !r.ctrl.snapshot().stopping));
    assert_eq!(rig.sim().stats().resets, 1);
    assert!(!rig.sim().spindle_on());
    assert_eq!(rig.sim().stats().beam_on_travel_mm, 0.0);
    assert!(rig.run_until(1000, |r| r.ctrl.snapshot().state() == MachineState::Alarm));
    assert!(matches!(rig.ctrl.start_job("x", square_job(1.0)), Err(Refusal::NotIdle(MachineState::Alarm))));
}

#[test]
fn error_mid_job_fails_at_its_line() {
    let mut rig = Rig::ready(SimOptions { error_at_line: Some(10), ..SimOptions::default() });
    let lines = square_job(20.0);
    rig.ctrl.start_job("square", lines.clone()).unwrap();
    assert!(rig.run_until(30_000, |r| matches!(r.job_state(), Some(JobState::Failed { .. }))));
    let Some(JobState::Failed { line, why }) = rig.job_state() else { panic!() };
    // The 10th G-code line the machine read is file line 11 (line 1 is a comment).
    assert_eq!(line, 11, "{why}");
    assert_eq!(lines[line - 1], "M5");
    assert!(why.contains("error 20"), "{why}");
    assert!(rig.run_until(5000, |r| !r.ctrl.snapshot().stopping));
    assert!(!rig.sim().spindle_on());
}

#[test]
fn disconnect_mid_job() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.start_job("long", long_job(500)).unwrap();
    assert!(rig.run_until(10_000, |r| r.ctrl.job().unwrap().acked > 20));
    rig.ctrl.on_disconnect();
    let Some(JobState::Failed { line, why }) = rig.job_state() else { panic!() };
    assert!(line > 20);
    assert!(why.contains("connection"));
    assert_eq!(rig.ctrl.phase(), Phase::Lost);
    assert!(rig.ctrl.poll(rig.now() + 1000).is_empty());
    rig.ctrl.on_bytes(b"ok\n<Idle|MPos:0,0,0>\n", rig.now());
    assert_eq!(rig.ctrl.command("$X"), Err(Refusal::NotReady));
}

#[test]
fn refuses_without_laser_mode() {
    let mut rig = Rig::ready(SimOptions { laser_mode: false, ..SimOptions::default() });
    let r = rig.ctrl.start_job("square", square_job(10.0));
    assert_eq!(r, Err(Refusal::LaserModeOff));
    assert!(r.unwrap_err().to_string().contains("$32=1"));
    // Turning it on through a command makes the next $$… but the controller only trusts what it
    // read: a reconnect reads it again.
    rig.ctrl.command("$32=1").unwrap();
    assert!(rig.run_until(1000, |r| r.sim().options().laser_mode));
    rig.ctrl.on_connected(rig.now());
    assert!(rig.run_until(10_000, |r| r.ctrl.phase() == Phase::Ready && r.ctrl.snapshot().status.is_some()));
    rig.ctrl.start_job("square", square_job(10.0)).unwrap();
}

#[test]
fn refusals() {
    let mut rig = Rig::new(SimOptions::default());
    assert_eq!(rig.ctrl.start_job("x", ["G0X1"]), Err(Refusal::NotReady));
    assert_eq!(rig.ctrl.command("$H"), Err(Refusal::NotReady));
    assert!(rig.run_until(10_000, |r| r.ctrl.phase() == Phase::Ready && r.ctrl.snapshot().status.is_some()));
    assert_eq!(rig.ctrl.start_job("x", ["; nothing", "  "]), Err(Refusal::Empty));
    assert_eq!(rig.ctrl.start_job("x", Vec::<String>::new()), Err(Refusal::Empty));
    assert!(matches!(rig.ctrl.start_job("x", ["G0X1", "G1X2F100", "G1 X1 ?"]), Err(Refusal::BadLine { line: 3, .. })));
    assert!(matches!(rig.ctrl.command("G0 X1 (oops"), Err(Refusal::BadLine { .. })));
    assert_eq!(rig.ctrl.command(""), Err(Refusal::Empty));
    // Not idle: a long jog.
    rig.ctrl.command("$J=G91 X100 F300").unwrap();
    assert!(rig.run_until(2000, |r| r.ctrl.snapshot().state() == MachineState::Jog));
    assert_eq!(rig.ctrl.start_job("x", ["G0X1"]), Err(Refusal::NotIdle(MachineState::Jog)));
    rig.ctrl.jog_cancel();
    assert!(rig.run_until(2000, |r| r.ctrl.snapshot().state() == MachineState::Idle));
    assert_eq!(rig.sim().stats().burn_mm, 0.0);
    rig.ctrl.start_job("x", ["G0X1"]).unwrap();
    assert_eq!(rig.ctrl.start_job("y", ["G0X1"]), Err(Refusal::JobActive));
}

#[test]
fn too_many_lines_refused_before_reading_them_all() {
    let mut rig = Rig::ready(SimOptions::default());
    let lines = std::iter::repeat_n("G0X1", crate::MAX_JOB_LINES + 1);
    assert_eq!(rig.ctrl.start_job("huge", lines), Err(Refusal::TooLong(crate::MAX_JOB_LINES + 1)));
}

#[test]
fn starts_in_alarm_then_unlocks_and_homes() {
    let mut rig = Rig::ready(SimOptions { homing: true, start_in_alarm: true, ..SimOptions::default() });
    assert_eq!(rig.ctrl.snapshot().state(), MachineState::Alarm);
    assert_eq!(rig.ctrl.start_job("x", ["G0X1"]), Err(Refusal::NotIdle(MachineState::Alarm)));
    rig.ctrl.command("$H").unwrap();
    assert!(rig.run_until(5000, |r| r.ctrl.snapshot().state() == MachineState::Idle));
    rig.ctrl.command("G0 X30 Y40").unwrap();
    rig.ctrl.command("G10 L20 P1 X0 Y0").unwrap();
    assert!(rig.run_until(5000, |r| {
        let s = r.ctrl.snapshot();
        s.status.as_ref().and_then(|st| st.wpos) == Some([0.0, 0.0, 0.0]) && s.status.as_ref().and_then(|st| st.mpos) == Some([30.0, 40.0, 0.0])
    }));
    rig.ctrl.command("$X").unwrap();
    assert!(rig.run_until(500, |r| r.log_has("Caution: Unlocked") || r.log_has("> $X")));
}

#[test]
fn a_machine_reset_mid_job_fails_it() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.start_job("long", long_job(500)).unwrap();
    assert!(rig.run_until(10_000, |r| r.ctrl.job().unwrap().acked > 10));
    // Someone presses the reset button on the controller.
    rig.link.sim_mut().receive(&[0x18]);
    assert!(rig.run_until(1000, |r| matches!(r.job_state(), Some(JobState::Failed { .. }))));
    let Some(JobState::Failed { why, .. }) = rig.job_state() else { panic!() };
    // GRBL reports a reset during motion as alarm 3 before its banner; a reset while still is
    // only the banner.
    assert!(why.contains("alarm 3") || why.contains("reset itself"), "{why}");
    assert!(!rig.ctrl.snapshot().stopping);
    assert!(rig.run_until(1000, |r| r.log_has("(M5 after the reset)")));
    assert_eq!(rig.sim().stats().resets, 1, "the banner ended the stop: no second reset");
}

#[test]
fn a_machine_reset_while_paused_fails_the_job() {
    let mut rig = Rig::ready(SimOptions::default());
    rig.ctrl.start_job("long", long_job(500)).unwrap();
    assert!(rig.run_until(10_000, |r| r.ctrl.job().unwrap().acked > 10));
    rig.ctrl.pause();
    assert!(rig.run_until(2000, |r| r.sim().state() == MachineState::Hold));
    rig.step(200); // hold complete: a reset now is not an alarm
    rig.link.sim_mut().receive(&[0x18]);
    assert!(rig.run_until(1000, |r| matches!(r.job_state(), Some(JobState::Failed { .. }))));
    let Some(JobState::Failed { why, .. }) = rig.job_state() else { panic!() };
    assert!(why.contains("reset itself"), "{why}");
    assert!(!rig.ctrl.snapshot().stopping);
}

#[test]
fn junk_from_the_machine_never_panics() {
    let mut rng = crate::proto_tests::Rng(11);
    let pieces: &[&[u8]] = &[
        b"ok\n",
        b"error:9\n",
        b"ALARM:2\n",
        b"<Idle|MPos:1,2,3|FS:0,0>\n",
        b"<Hold:0|WPos:1,2,3>\n",
        b"Grbl 1.1h ['$' for help]\n",
        b"$32=1\n",
        b"[OPT:V,15,7]\n",
        b"[VER:x]\n",
        b"\xff\xfe\x00\r\n",
        b"<Run|MPos:nan",
        b"\n",
    ];
    let mut ctrl = Controller::new();
    ctrl.on_connected(0);
    let mut now = 0;
    for i in 0..20_000 {
        let p = pieces[rng.below(pieces.len() as u64) as usize];
        ctrl.on_bytes(p, now);
        if i % 7 == 0 {
            let _ = ctrl.start_job("j", ["G0X1", "G1X2F100"]);
        }
        if i % 13 == 0 {
            ctrl.stop();
        }
        if i % 17 == 0 {
            ctrl.pause();
            ctrl.resume();
        }
        let _ = ctrl.command("$X");
        let out = ctrl.poll(now);
        assert!(out.len() < 4096);
        now += rng.below(100);
    }
    // An over-long line is dropped, not kept.
    ctrl.on_bytes(&[b'x'; 10_000], now);
    ctrl.on_bytes(b"\n", now);
    assert!(ctrl.snapshot().log.len() <= crate::LOG_LINES);
    assert!(ctrl.snapshot().log.iter().all(|l| l.len() <= 120));
}

#[test]
fn throughput_100k_lines() {
    let mut rig = Rig::ready(SimOptions { speed_factor: 100_000.0, rapid_mm_min: 10_000.0, ..SimOptions::default() });
    let lines = long_job(33_332); // 4 + 3 × 33 332 + 2 = 100 002 lines
    let n = lines.len();
    assert!(n >= 100_000);
    let started = std::time::Instant::now();
    let sim_start = rig.now();
    rig.ctrl.start_job("100k", lines).unwrap();
    assert!(rig.run_until(10_000_000, |r| r.job_state() == Some(JobState::Done)));
    let wall = started.elapsed().as_secs_f64();
    let sim_ms = rig.now() - sim_start;
    let st = rig.sim().stats();
    assert_eq!(st.beam_on_travel_mm, 0.0);
    assert_eq!(st.overflows, 0);
    assert!(st.max_rx_used <= 128);
    eprintln!(
        "throughput: {n} lines, wall {wall:.3} s ({:.0} lines/s), simulated {sim_ms} ms ({:.0} lines/s of link time), max rx {} bytes",
        n as f64 / wall,
        n as f64 / (sim_ms as f64 / 1000.0),
        st.max_rx_used
    );
}
