use crate::proto::{cap_str, parse_status};
use crate::{MAX_LINE, MachineState, Response, Settings, alarm_advice, alarm_text, clean_line, error_text, parse_line};

/// A tiny deterministic generator for junk (SplitMix64).
pub(crate) struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

#[test]
fn responses() {
    assert_eq!(parse_line("ok\r\n"), Response::Ok);
    assert_eq!(parse_line("error:22"), Response::Error(22));
    assert_eq!(parse_line("ALARM:1"), Response::Alarm(1));
    assert_eq!(parse_line("Grbl 1.1h ['$' for help]"), Response::Welcome { version: "1.1h".into() });
    assert_eq!(parse_line("\u{0}\u{7f}Grbl 1.1f ['$' for help]"), Response::Welcome { version: "1.1f".into() });
    assert_eq!(parse_line("$32=1"), Response::Setting(32, "1".into()));
    assert_eq!(parse_line("$130=584.200"), Response::Setting(130, "584.200".into()));
    assert_eq!(parse_line("[MSG:'$H'|'$X' to unlock]"), Response::Message("'$H'|'$X' to unlock".into()));
    assert_eq!(parse_line("[VER:1.1h.20190825:]"), Response::Feedback("VER:1.1h.20190825:".into()));
    assert_eq!(parse_line("[OPT:V,15,128]"), Response::Feedback("OPT:V,15,128".into()));
    assert_eq!(parse_line("[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]"), Response::Feedback("GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0".into()));
    assert_eq!(parse_line(">G54G20:ok"), Response::Other(">G54G20:ok".into()));
    assert_eq!(parse_line("$N0="), Response::Other("$N0=".into()));
    assert_eq!(parse_line("error:x"), Response::Other("error:x".into()));
    assert_eq!(parse_line("error:99999999"), Response::Other("error:99999999".into()));
    assert_eq!(parse_line("okay"), Response::Other("okay".into()));
}

#[test]
fn status_reports() {
    let Response::Status(s) = parse_line("<Idle|MPos:1.000,2.500,-3.000|FS:0,0|WCO:0.000,0.000,0.000>") else { panic!() };
    assert_eq!(s.state, MachineState::Idle);
    assert_eq!(s.mpos, Some([1.0, 2.5, -3.0]));
    assert_eq!(s.wco, Some([0.0; 3]));
    assert_eq!((s.feed, s.spindle), (Some(0.0), Some(0.0)));
    let Response::Status(s) = parse_line("<Hold:1|WPos:5,6,7|Bf:15,128|Ln:99|F:500|Ov:100,50,120|Pn:XYZ|A:SF>") else { panic!() };
    assert_eq!((s.state, s.sub), (MachineState::Hold, Some(1)));
    assert_eq!(s.wpos, Some([5.0, 6.0, 7.0]));
    assert_eq!(s.mpos, None);
    assert_eq!(s.buffer, Some((15, 128)));
    assert_eq!(s.line, Some(99));
    assert_eq!(s.feed, Some(500.0));
    assert_eq!(s.ov, Some([100, 50, 120]));
    assert_eq!(s.pins, "XYZ");
    assert_eq!(s.accessories, "SF");
    // Two axes (an XY build), unknown state, unknown fields ignored.
    let s = parse_status("Weird|MPos:1,2|Zz:9").unwrap();
    assert_eq!(s.state, MachineState::Unknown);
    assert_eq!(s.mpos, Some([1.0, 2.0, 0.0]));
    for st in ["Idle", "Run", "Hold", "Jog", "Alarm", "Door", "Check", "Home", "Sleep"] {
        assert_eq!(MachineState::parse(st).as_str(), st);
    }
}

#[test]
fn status_junk() {
    for bad in ["<>", "<|>", "<Idle|MPos:nan,1,2>", "<Idle|MPos:1e400,0,0>", "<Run|MPos:1,2,3,4,5,6,7>", "<Run|FS:x,y|Ov:1|Bf:-5,1e30>"] {
        if let Response::Status(s) = parse_line(bad) {
            assert!(s.mpos.is_none(), "{bad}");
            assert!(s.ov.is_none());
            assert!(s.feed.is_none());
            if let Some((a, b)) = s.buffer {
                assert_eq!(a, 0);
                assert_eq!(b, u32::MAX);
            }
        }
    }
    // Truncated.
    assert!(matches!(parse_line("<Idle|MPos:1.0,2"), Response::Other(_)));
    let long = format!("<Idle|Pn:{}>", "X".repeat(1000));
    let _ = parse_line(&long);
}

#[test]
fn parsers_never_panic_on_junk() {
    let mut rng = Rng(7);
    let alphabet: &[u8] = b"<>|:,.-0123456789okerrorALARM$=[]MSGGrbl 'for help]IdleRunHoldMPosWPosWCOFSBfLnOvPnA\xff\xfe\x00\x18";
    for _ in 0..20_000 {
        let len = rng.below(120) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect();
        let text = String::from_utf8_lossy(&bytes);
        let _ = parse_line(&text);
        let _ = clean_line(&text);
        let mut s = Settings::default();
        if let Response::Setting(n, v) = parse_line(&text) {
            s.set(n, &v);
        }
    }
    for code in 0..=u16::MAX {
        assert!(!error_text(code).is_empty());
        assert!(!alarm_text(code).is_empty());
        assert!(!alarm_advice(code).is_empty());
    }
    assert_eq!(cap_str("ééé", 3), "é");
}

#[test]
fn cleaning_lines() {
    assert_eq!(clean_line("G1 X10.5 Y2 ; cut").unwrap(), "G1X10.5Y2");
    assert_eq!(clean_line("(header) G0 X0 (mid) Y0").unwrap(), "G0X0Y0");
    assert_eq!(clean_line("   ").unwrap(), "");
    assert_eq!(clean_line("; only a comment").unwrap(), "");
    assert_eq!(clean_line("$J=G91 X10 F1000").unwrap(), "$J=G91X10F1000");
    assert!(clean_line("G1 X1 (open").is_err());
    assert!(clean_line("G1 X1 ?").is_err());
    assert!(clean_line("G1 X1 !").is_err());
    assert!(clean_line("G1 X1 ~").is_err());
    assert!(clean_line("G1 Xé").is_err());
    assert!(clean_line("G1\u{1}X1").is_err());
    assert!(clean_line(&"X".repeat(MAX_LINE)).is_ok());
    assert!(clean_line(&"X".repeat(MAX_LINE + 1)).is_err());
    // Whitespace and comments don't count toward the limit.
    assert!(clean_line(&format!("{} ; {}", "X".repeat(MAX_LINE), "c".repeat(200))).is_ok());
}

#[test]
fn settings() {
    let mut s = Settings::default();
    for (n, v) in [(10, "1"), (20, "0"), (22, "1"), (30, "1000"), (32, "1"), (130, "584.2"), (131, "1219.2"), (132, "50")] {
        assert!(s.set(n, v));
    }
    assert!(!s.set(1, "abc"));
    assert!(!s.set(1, "inf"));
    assert_eq!(s.laser_mode(), Some(true));
    assert_eq!(s.s_max(), Some(1000.0));
    assert_eq!(s.max_travel(), Some([584.2, 1219.2, 50.0]));
    assert_eq!(s.homing(), Some(true));
    assert_eq!(s.soft_limits(), Some(false));
    assert_eq!(s.report_mpos(), Some(true));
    s.set(10, "2");
    assert_eq!(s.report_mpos(), Some(false));
    s.set(32, "0");
    assert_eq!(s.laser_mode(), Some(false));
    assert_eq!(Settings::default().max_travel(), None);
    for n in 0..1000 {
        s.set(n, "1");
    }
    assert!(s.values.len() <= 256);
}
