use crate::sim::{SimGrbl, SimOptions};
use crate::{MachineState, Response, parse_line};

fn drain(sim: &mut SimGrbl) -> Vec<String> {
    let mut buf = vec![0u8; 1 << 16];
    let n = sim.read_out(&mut buf);
    String::from_utf8_lossy(&buf[..n]).lines().map(str::to_string).filter(|l| !l.is_empty()).collect()
}

fn send(sim: &mut SimGrbl, lines: &[&str]) -> Vec<String> {
    for l in lines {
        sim.receive(format!("{l}\n").as_bytes());
    }
    drain(sim)
}

fn quiet(opt: SimOptions) -> SimGrbl {
    let mut sim = SimGrbl::new(opt);
    drain(&mut sim);
    sim
}

#[test]
fn banner_settings_and_info() {
    let mut sim = SimGrbl::new(SimOptions::default());
    let out = drain(&mut sim);
    assert_eq!(parse_line(&out[0]), Response::Welcome { version: "1.1h".into() });
    let out = send(&mut sim, &["$$"]);
    assert!(out.contains(&"$32=1.000".to_string()), "{out:?}");
    assert!(out.contains(&"$30=1000.000".to_string()));
    assert_eq!(out.last().map(String::as_str), Some("ok"));
    let out = send(&mut sim, &["$I"]);
    assert_eq!(out, ["[VER:1.1h.SIM:]", "[OPT:V,15,128]", "ok"]);
    // No banner when the board doesn't reset on open.
    let mut sim = SimGrbl::new(SimOptions { banner_on_open: false, ..SimOptions::default() });
    assert!(drain(&mut sim).is_empty());
    sim.receive(&[0x18]);
    assert!(drain(&mut sim).iter().any(|l| l.starts_with("Grbl 1.1h")));
}

#[test]
fn moves_at_feed_and_reports_status() {
    let mut sim = quiet(SimOptions::default());
    let out = send(&mut sim, &["G21G90", "G1X10F600"]);
    assert_eq!(out, ["ok", "ok"]);
    assert_eq!(sim.state(), MachineState::Run);
    sim.advance(500); // 600 mm/min = 10 mm/s: 5 mm in 0.5 s
    assert!((sim.mpos()[0] - 5.0).abs() < 1e-6, "{:?}", sim.mpos());
    sim.receive(b"?");
    let out = drain(&mut sim);
    let Response::Status(s) = parse_line(&out[0]) else { panic!("{out:?}") };
    assert_eq!(s.state, MachineState::Run);
    assert_eq!(s.feed, Some(600.0));
    assert!(s.buffer.is_some());
    sim.advance(600);
    assert_eq!(sim.state(), MachineState::Idle);
    assert!((sim.mpos()[0] - 10.0).abs() < 1e-9);
    // Feed rate missing.
    let mut sim = quiet(SimOptions::default());
    assert_eq!(send(&mut sim, &["G1X5"]), ["error:22"]);
    assert_eq!(send(&mut sim, &["G7"]), ["error:20"]);
    assert_eq!(send(&mut sim, &["X"]), ["error:2"]);
    assert_eq!(send(&mut sim, &["5"]), ["error:1"]);
    assert_eq!(send(&mut sim, &[&"G0X1".repeat(30)]), ["error:11"]);
}

#[test]
fn laser_mode_g0_never_lit() {
    let mut sim = quiet(SimOptions::default());
    send(&mut sim, &["M4S500", "G0X10Y10", "G1X20F600", "G0X0Y0", "G1Y10", "M5"]);
    sim.advance(60_000);
    let st = sim.stats();
    assert_eq!(st.beam_on_travel_mm, 0.0);
    assert!((st.burn_mm - 20.0).abs() < 1e-6, "{st:?}");
    assert_eq!(st.beam_on_still_ms, 0.0, "M4 is off when standing still");
    assert!(!sim.beam_on());
}

#[test]
fn laser_mode_off_lights_travel_and_dwells() {
    let mut sim = quiet(SimOptions { laser_mode: false, ..SimOptions::default() });
    send(&mut sim, &["M3S500", "G0X10", "G1X20F600"]);
    sim.advance(10_000);
    let st = sim.stats().clone();
    assert!((st.beam_on_travel_mm - 10.0).abs() < 1e-6, "{st:?}");
    assert!(st.beam_on_still_ms > 0.0);
    assert!(sim.beam_on());
    // S0 is off even with M3.
    let mut sim = quiet(SimOptions { laser_mode: false, ..SimOptions::default() });
    send(&mut sim, &["M3S0", "G0X10"]);
    sim.advance(10_000);
    assert_eq!(sim.stats().burn_mm, 0.0);
}

#[test]
fn m3_in_laser_mode_stays_lit_when_still() {
    let mut sim = quiet(SimOptions::default());
    send(&mut sim, &["M3S100", "G1X1F600"]);
    sim.advance(5_000);
    assert!(sim.stats().beam_on_still_ms > 0.0);
    assert!(sim.beam_on());
    send(&mut sim, &["M5"]);
    assert!(!sim.beam_on());
}

#[test]
fn hold_resume_and_reset() {
    let mut sim = quiet(SimOptions::default());
    send(&mut sim, &["M4S500", "G1X100F600"]);
    sim.advance(1000);
    sim.receive(b"!");
    sim.advance(10);
    sim.receive(b"?");
    assert!(drain(&mut sim)[0].starts_with("<Hold:1"));
    sim.advance(100);
    sim.receive(b"?");
    assert!(drain(&mut sim)[0].starts_with("<Hold:0"));
    let x = sim.mpos()[0];
    sim.advance(1000);
    assert_eq!(sim.mpos()[0], x, "held");
    assert!(!sim.beam_on(), "beam off in a hold");
    sim.receive(b"~");
    sim.advance(100);
    assert!(sim.mpos()[0] > x);
    // Reset while moving: alarm 3, buffers purged, spindle off.
    sim.receive(b"G1X200\n");
    sim.receive(&[0x18]);
    let out = drain(&mut sim);
    assert!(out.contains(&"ALARM:3".to_string()), "{out:?}");
    assert!(out.iter().any(|l| l.starts_with("Grbl")));
    assert_eq!(sim.state(), MachineState::Alarm);
    assert_eq!(sim.planner_len(), 0);
    assert_eq!(sim.rx_used(), 0);
    assert!(!sim.spindle_on());
    assert_eq!(send(&mut sim, &["G0X0"]), ["error:9"]);
    assert_eq!(send(&mut sim, &["$X"]), ["[MSG:Caution: Unlocked]", "ok"]);
    assert_eq!(sim.state(), MachineState::Idle);
}

#[test]
fn reset_when_still_keeps_idle() {
    let mut sim = quiet(SimOptions::default());
    send(&mut sim, &["G1X5F6000"]);
    sim.advance(1000);
    sim.receive(&[0x18]);
    let out = drain(&mut sim);
    assert!(!out.contains(&"ALARM:3".to_string()));
    assert_eq!(sim.state(), MachineState::Idle);
    assert_eq!(sim.mpos()[0], 5.0);
}

#[test]
fn receive_buffer_overflow_is_recorded() {
    let mut sim = quiet(SimOptions::default());
    // Fill the planner so nothing more is read, then push past the 128 bytes.
    for i in 0..15 {
        sim.receive(format!("G1X{}F60\n", i + 1).as_bytes());
    }
    assert_eq!(sim.planner_len(), 15);
    sim.receive(&[b'G'; 128]);
    assert_eq!(sim.rx_used(), 128);
    assert_eq!(sim.stats().overflows, 0);
    sim.receive(b"G");
    assert_eq!(sim.stats().overflows, 1);
}

#[test]
fn planner_full_holds_back_oks() {
    let mut sim = quiet(SimOptions::default());
    let lines: Vec<String> = (0..20).map(|i| format!("G1X{}F60", i + 1)).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let out = send(&mut sim, &refs);
    assert_eq!(out.len(), 15, "only the planner's 15 blocks answered");
    sim.advance(2000);
    assert!(!drain(&mut sim).is_empty());
}

#[test]
fn dwell_answers_after_motion() {
    let mut sim = quiet(SimOptions::default());
    let out = send(&mut sim, &["G1X10F600", "G4P0.5"]);
    assert_eq!(out, ["ok"], "the dwell waits for the move");
    sim.advance(1000);
    assert!(drain(&mut sim).is_empty(), "move done at 1 s, then 0.5 s dwell");
    sim.advance(600);
    assert_eq!(drain(&mut sim), ["ok"]);
}

#[test]
fn alarms_jog_homing_origin() {
    let mut sim = quiet(SimOptions { homing: true, start_in_alarm: true, ..SimOptions::default() });
    assert_eq!(sim.state(), MachineState::Alarm);
    assert_eq!(send(&mut sim, &["$J=G91X10F600"]), ["error:9"]);
    assert_eq!(send(&mut sim, &["$H"]), Vec::<String>::new());
    assert_eq!(sim.state(), MachineState::Home);
    sim.advance(2000);
    assert_eq!(drain(&mut sim), ["ok"]);
    assert_eq!(sim.state(), MachineState::Idle);
    assert_eq!(send(&mut sim, &["$J=G91X10F600"]), ["ok"]);
    assert_eq!(sim.state(), MachineState::Jog);
    assert_eq!(send(&mut sim, &["$J=G91M3X10F600"]), ["error:16"]);
    sim.receive(&[0x85]);
    assert_eq!(sim.state(), MachineState::Idle);
    sim.advance(10);
    assert_eq!(send(&mut sim, &["G0X20Y30"]), ["ok"]);
    sim.advance(10_000);
    assert_eq!(send(&mut sim, &["G10L20P1X0Y0"]), ["ok"]);
    assert_eq!(sim.wpos(), [0.0, 0.0, 0.0]);
    assert_eq!(sim.mpos()[1], 30.0);
    sim.raise_alarm(1);
    let out = drain(&mut sim);
    assert_eq!(out, ["ALARM:1", "[MSG:Reset to continue]"]);
    sim.receive(b"?");
    assert!(drain(&mut sim).is_empty(), "critical alarm: no status until reset");
    sim.receive(&[0x18]);
    assert!(drain(&mut sim).iter().any(|l| l.contains("to unlock")));
    // Homing disabled.
    let mut sim = quiet(SimOptions::default());
    assert_eq!(send(&mut sim, &["$H"]), ["error:5"]);
    assert_eq!(send(&mut sim, &["$32=0"]), ["ok"]);
    assert!(!sim.options().laser_mode);
    assert_eq!(send(&mut sim, &["$999=1"]), ["error:3"]);
}

#[test]
fn junk_never_panics() {
    let mut rng = crate::proto_tests::Rng(3);
    let mut sim = quiet(SimOptions { speed_factor: 50.0, ..SimOptions::default() });
    let alphabet = b"GMXYZFSPLN0123456789.-$=J?!~\n\n\n \x18\x85\xff";
    for _ in 0..20_000 {
        let n = rng.below(20) as usize;
        let bytes: Vec<u8> = (0..n).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect();
        sim.receive(&bytes);
        sim.advance(rng.below(30));
        drain(&mut sim);
    }
}
