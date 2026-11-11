//! PICABIA: drag-knife G-code: lifts and plunges, blade offset and swivel arcs, overcut, passes,
//! no spindle words, engraving left out, params, random sheets.

use serde_json::json;
use vectorcraft_geom::kurbo::{Point, Vec2};

use crate::fixture::Rng;
use crate::{Gcode, GcodeParams, Head, KnifeParams, LaserSheet, knife_gcode, ladder_gcode, laser_gcode, power_ladder};

fn pts(v: &[(f64, f64)]) -> Vec<Point> {
    v.iter().map(|&(x, y)| Point::new(x, y)).collect()
}

fn sheet(w: f64, h: f64, cut: Vec<Vec<Point>>) -> LaserSheet {
    LaserSheet { w_mm: w, h_mm: h, cut, name: "1 of 1".into(), ..LaserSheet::default() }
}

fn square(x: f64, y: f64, s: f64) -> Vec<Point> {
    pts(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)])
}

fn knife(k: KnifeParams) -> GcodeParams {
    GcodeParams { head: Head::Knife, knife: k, ..GcodeParams::default() }
}

/// No offset, no overcut: the path as drawn.
fn plain() -> KnifeParams {
    KnifeParams { offset_mm: 0.0, overcut_mm: 0.0, ..KnifeParams::default() }
}

fn code(g: &Gcode) -> Vec<&str> {
    g.text.lines().filter(|l| !l.starts_with(';')).collect()
}

fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace().find_map(|w| w.strip_prefix(letter)).and_then(|v| v.parse().ok())
}

/// The machine as the file drives it: every move with the Z it happens at.
#[derive(Debug, PartialEq)]
struct Move {
    g: u8,
    from: Point,
    to: Point,
    z: f64,
    /// Arcs: the centre.
    centre: Option<Point>,
}

/// Plays the file, checking what every knife file must do: only the allowed words, no spindle or
/// laser on, Z lifted to `safe` before the first move across the sheet and before every `G0`
/// across it, the blade down (Z ≤ 0) for every `G1`/`G2`/`G3` across it, arcs never whole circles
/// and with matching radii (GRBL's error 33), up and parked at the end, `M5` last.
fn play(g: &Gcode, safe: f64) -> Vec<Move> {
    assert!(!g.text.is_empty(), "{:?}", g.warnings);
    assert!(!g.text.contains('\r'));
    assert_eq!(g.lines, g.text.lines().count());
    let mut at = Point::ZERO;
    let mut z: Option<f64> = None;
    let mut out = Vec::new();
    for l in code(g) {
        assert!(l.len() <= 80 && l.is_ascii(), "{l}");
        for w in l.split_whitespace() {
            let (letter, value) = w.split_at(1);
            assert!(!value.contains(['e', 'E']) && value != "-0" && value.parse::<f64>().is_ok(), "number {w} in {l}");
            match letter {
                "G" => assert!(["0", "1", "2", "3", "21", "90"].contains(&value), "{l}"),
                "M" => assert_eq!(value, "5", "only M5: {l}"),
                "F" | "X" | "Y" | "Z" | "I" | "J" => {}
                _ => panic!("word {w} in {l}"),
            }
        }
        let Some(gn) = word(l, 'G').filter(|_| l.starts_with("G0 ") || l.starts_with("G1 ") || l.starts_with("G2 ") || l.starts_with("G3 ")) else {
            continue;
        };
        let gn = gn as u8;
        if let Some(nz) = word(l, 'Z') {
            assert!(word(l, 'X').is_none() && word(l, 'Y').is_none(), "Z moves alone: {l}");
            if gn == 0 {
                assert_eq!(nz, safe, "rapids go up to the travel height: {l}");
            } else {
                assert_eq!(gn, 1, "{l}");
                assert!(nz <= 0.0 && z == Some(safe), "plunges go from the travel height into the sheet: {l}");
            }
            z = Some(nz);
            continue;
        }
        let to = Point::new(word(l, 'X').unwrap_or(at.x), word(l, 'Y').unwrap_or(at.y));
        let z_now = z.expect("Z set before any move across the sheet");
        let centre = if gn >= 2 {
            let c = Point::new(at.x + word(l, 'I').unwrap(), at.y + word(l, 'J').unwrap());
            assert_ne!(to, at, "an arc onto its start is a whole circle: {l}");
            assert!(((to - c).hypot() - (at - c).hypot()).abs() < 0.005, "arc radii differ: {l}");
            Some(c)
        } else {
            None
        };
        if gn == 0 {
            assert_eq!(z_now, safe, "travel only at the travel height: {l}");
        } else {
            assert!(z_now <= 0.0, "cut only with the blade down: {l}");
        }
        out.push(Move { g: gn, from: at, to, z: z_now, centre });
        at = to;
    }
    let c = code(g);
    let n = c.len();
    assert_eq!(c[n - 1], "M5");
    assert!(c[n - 2].starts_with("G0 X"), "park: {}", c[n - 2]);
    assert_eq!(z, Some(safe), "up at the end");
    let first = c.iter().position(|l| l.starts_with('G') && !l.starts_with("G2") && !l.starts_with("G9")).unwrap();
    assert!(c[..first].contains(&"M5"), "M5 before the first motion");
    out
}

#[test]
fn a_plain_square_is_lifted_plunged_cut_and_parked() {
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let g = knife_gcode(&s, &knife(plain()));
    play(&g, 5.0);
    assert_eq!(
        code(&g),
        vec![
            "G21",
            "G90",
            "M5",
            "G0 Z5",
            "G0 X10 Y40",
            "G1 Z-0.5 F300",
            "G1 X20 Y40 F1500",
            "G1 X20 Y30",
            "G1 X10 Y30",
            "G1 X10 Y40",
            "G0 Z5",
            "G0 X0 Y0",
            "M5",
        ]
    );
    assert!((g.burn_mm - 40.0).abs() < 1e-9);
    // 40 mm at 1500 mm/min plus a 5.5 mm plunge at 300.
    assert!((g.burn_sec - (1.6 + 1.1)).abs() < 1e-9, "{}", g.burn_sec);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
}

#[test]
fn the_blade_offset_leads_every_point_and_corners_swivel_round_them() {
    let o = 0.25;
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let g = knife_gcode(&s, &knife(KnifeParams { offset_mm: o, overcut_mm: 0.0, ..KnifeParams::default() }));
    let m = play(&g, 5.0);
    // Work coordinates: (10,40) → (20,40) → (20,30) → (10,30) → back. The axis starts `o` along
    // the first side, ahead of the corner.
    let start = m.iter().find(|m| m.g == 0).unwrap().to;
    assert_eq!(start, Point::new(10.25, 40.0));
    let arcs: Vec<&Move> = m.iter().filter(|m| m.g >= 2).collect();
    assert_eq!(arcs.len(), 3, "three corners turn 90°: {m:?}");
    let corners = [Point::new(20.0, 40.0), Point::new(20.0, 30.0), Point::new(10.0, 30.0)];
    for (a, c) in arcs.iter().zip(corners) {
        assert_eq!(a.centre, Some(c), "the blade turns on the corner");
        assert_eq!(a.g, 2, "the square runs clockwise in work coordinates: G2");
        assert!(((a.from - c).hypot() - o).abs() < 1e-9 && ((a.to - c).hypot() - o).abs() < 1e-9);
    }
    // The lines are the sides shifted `o` along their own direction.
    let lines: Vec<&Move> = m.iter().filter(|m| m.g == 1).collect();
    assert_eq!((lines[0].from, lines[0].to), (Point::new(10.25, 40.0), Point::new(20.25, 40.0)));
    assert_eq!((lines[1].from, lines[1].to), (Point::new(20.0, 39.75), Point::new(20.0, 29.75)));
    assert_eq!(lines.last().unwrap().to, Point::new(10.0, 40.25), "ends `o` past the start, going up");
}

#[test]
fn a_gentle_corner_gets_a_chord_and_the_turn_sets_g2_or_g3() {
    // An open score line turning left by 10° then right by 60° (y down on the sheet).
    let turn = |deg: f64| Vec2::new(deg.to_radians().cos(), -deg.to_radians().sin());
    let a = Point::new(10.0, 40.0);
    let b = a + Vec2::new(10.0, 0.0);
    let c = b + turn(10.0) * 10.0;
    let d = c + turn(-50.0) * 10.0;
    let mut s = sheet(100.0, 50.0, Vec::new());
    s.score = vec![vec![a, b, c, d]];
    let k = KnifeParams { offset_mm: 0.5, swivel_deg: 15.0, ..KnifeParams::default() };
    let m = play(&knife_gcode(&s, &knife(k)), 5.0);
    let arcs: Vec<&Move> = m.iter().filter(|m| m.g >= 2).collect();
    assert_eq!(arcs.len(), 1, "only the 60° corner swivels: {m:?}");
    // On the sheet (y down) the second corner turns right; flipped to Y up it is clockwise.
    assert_eq!(arcs[0].g, 2);
    assert!(m.iter().all(|m| m.z == -0.2 || m.g == 0), "score lines at the score depth");
}

#[test]
fn closed_contours_overcut_past_their_start() {
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let g = knife_gcode(&s, &knife(KnifeParams { offset_mm: 0.0, overcut_mm: 3.0, ..KnifeParams::default() }));
    let m = play(&g, 5.0);
    let cut: Vec<Point> = m.iter().filter(|m| m.g == 1).map(|m| m.to).collect();
    assert_eq!(cut.last(), Some(&Point::new(13.0, 40.0)), "3 mm along the first side again: {cut:?}");
    assert!((g.burn_mm - 43.0).abs() < 1e-9);
    // An overcut longer than a side carries on round the next corner.
    let small = sheet(100.0, 50.0, vec![square(10.0, 10.0, 5.0)]);
    let g = knife_gcode(&small, &knife(KnifeParams { offset_mm: 0.0, overcut_mm: 7.0, ..KnifeParams::default() }));
    let cut: Vec<Point> = play(&g, 5.0).iter().filter(|m| m.g == 1).map(|m| m.to).collect();
    assert_eq!(&cut[cut.len() - 2..], &[Point::new(15.0, 40.0), Point::new(15.0, 38.0)]);
}

#[test]
fn passes_lift_and_plunge_each_time_and_score_runs_before_cut() {
    let mut s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0), square(40.0, 10.0, 5.0)]);
    s.score = vec![pts(&[(0.0, 25.0), (100.0, 25.0)])];
    let g = knife_gcode(&s, &knife(KnifeParams { passes: 3, ..plain() }));
    play(&g, 5.0);
    let c = code(&g);
    let plunges: Vec<&str> = c.iter().copied().filter(|l| l.starts_with("G1 Z")).collect();
    assert_eq!(plunges.len(), 1 + 2 * 3, "the score once, each contour three times");
    assert!(plunges[0].starts_with("G1 Z-0.2"), "score first, shallow");
    assert!(plunges[1..].iter().all(|l| l.starts_with("G1 Z-0.5")));
    // Score off: left out.
    let g = knife_gcode(&s, &knife(KnifeParams { score: false, ..plain() }));
    assert!(!g.text.contains("Z-0.2"));
}

#[test]
fn engraving_is_left_out_with_a_warning_and_nothing_to_cut_is_refused() {
    let mut s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    s.engrave = vec![pts(&[(1.0, 1.0), (2.0, 2.0)]), pts(&[(3.0, 3.0), (4.0, 3.0)])];
    let g = knife_gcode(&s, &knife(plain()));
    play(&g, 5.0);
    assert!(g.warnings.iter().any(|w| w.contains("doesn't engrave") && w.contains('2')), "{:?}", g.warnings);
    s.cut.clear();
    let g = knife_gcode(&s, &knife(plain()));
    assert!(g.text.is_empty() && g.warnings.iter().any(|w| w.contains("nothing to cut")), "{:?}", g.warnings);
}

#[test]
fn the_laser_writer_ignores_the_head_and_the_ladder_refuses_the_knife() {
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let laser = laser_gcode(&s, &GcodeParams::default());
    assert_eq!(laser_gcode(&s, &knife(plain())).text, laser.text);
    assert!(!laser.text.contains('Z'));
    let l = power_ladder(&crate::LadderParams::default());
    let g = ladder_gcode(&l, &knife(plain()));
    assert!(g.text.is_empty() && g.warnings[0].contains("power ladder"), "{:?}", g.warnings);
}

#[test]
fn the_origin_offsets_every_move_and_the_header_says_z0_is_the_surface() {
    let s = sheet(100.0, 50.0, vec![square(10.0, 10.0, 10.0)]);
    let p = GcodeParams { origin_mm: Point::new(5.0, 7.0), header: vec!["machine: X-Carve".into()], ..knife(plain()) };
    let g = knife_gcode(&s, &p);
    let m = play(&g, 5.0);
    assert_eq!(m[0].to, Point::new(15.0, 47.0));
    assert_eq!(m.last().unwrap().to, Point::new(5.0, 7.0), "park at the origin");
    let header: Vec<&str> = g.text.lines().filter(|l| l.starts_with(';')).collect();
    assert!(header.iter().any(|l| l.contains("Z0 is the sheet's surface")));
    assert!(header.iter().any(|l| l.contains("machine: X-Carve")));
    assert!(header.iter().all(|l| l.len() <= 70));
}

#[test]
fn knife_params_clamp_and_read_json_leniently() {
    let d = KnifeParams::default();
    assert_eq!((d.safe_z_mm, d.cut_z_mm, d.score_z_mm, d.offset_mm, d.passes), (5.0, -0.5, -0.2, 0.25, 1));
    let k = KnifeParams {
        safe_z_mm: 0.0,
        cut_z_mm: 3.0,
        score: false,
        score_z_mm: f64::NAN,
        feed_mm_min: 1e9,
        plunge_mm_min: -1.0,
        passes: 0,
        offset_mm: 9.0,
        swivel_deg: 400.0,
        overcut_mm: f64::INFINITY,
    }
    .clamped();
    assert_eq!((k.safe_z_mm, k.cut_z_mm, k.score_z_mm, k.feed_mm_min, k.plunge_mm_min), (0.5, 0.0, -0.2, 20_000.0, 1.0));
    assert_eq!((k.passes, k.offset_mm, k.swivel_deg, k.overcut_mm, k.score), (1, 2.0, 180.0, 1.0, false));
    // Through GcodeParams: `head` and `knife`, overlaid field by field; junk keeps the value.
    let p = GcodeParams::from_json(&json!({"head": " Knife ", "knife": {"cutZMm": -0.8, "passes": 2.6, "offsetMm": "x"}}));
    assert_eq!((p.head, p.knife.cut_z_mm, p.knife.passes, p.knife.offset_mm), (Head::Knife, -0.8, 3, 0.25));
    assert_eq!(GcodeParams::from_json(&json!({"head": "router"})).head, Head::Laser);
    assert_eq!(GcodeParams::from_json(&p.to_json()), GcodeParams { header: Vec::new(), ..p.clone() });
    assert_eq!(p.to_json()["head"], json!("knife"));
}

/// Random sheets and junk params: the file is always either refused with a reason or safe.
#[test]
fn random_sheets_always_give_safe_knife_gcode() {
    let mut rng = Rng::new(0x6b6e_6966);
    let junk = [f64::NAN, f64::INFINITY, -1e300, 1e30, -0.0];
    for round in 0..300 {
        let w = rng.range(1.0, 800.0);
        let h = rng.range(1.0, 800.0);
        let point = |rng: &mut Rng| {
            if rng.next().is_multiple_of(40) {
                let j = junk[(rng.next() % junk.len() as u64) as usize];
                Point::new(j, rng.range(0.0, h))
            } else if rng.next().is_multiple_of(6) {
                // Nearly repeated points: rounding can fold them together.
                Point::new(rng.range(0.0, 0.002), rng.range(0.0, 0.002))
            } else {
                Point::new(rng.range(-5.0, w + 5.0), rng.range(-5.0, h + 5.0))
            }
        };
        let paths =
            |rng: &mut Rng, n: u64| -> Vec<Vec<Point>> { (0..rng.next() % n).map(|_| (0..rng.next() % 12).map(|_| point(rng)).collect()).collect() };
        let mut s = sheet(w, h, paths(&mut rng, 8));
        s.score = paths(&mut rng, 3);
        s.engrave = paths(&mut rng, 3);
        let any = |rng: &mut Rng, lo: f64, hi: f64| -> serde_json::Value {
            match rng.next() % 6 {
                0 => json!("junk"),
                1 => json!(null),
                2 => json!(-1e300),
                3 => json!(1e300),
                _ => json!(rng.range(lo, hi)),
            }
        };
        let v = json!({
            "head": "knife",
            "originXMm": any(&mut rng, -6000.0, 6000.0),
            "originYMm": any(&mut rng, -6000.0, 6000.0),
            "knife": {
                "safeZMm": any(&mut rng, 0.0, 60.0),
                "cutZMm": any(&mut rng, -12.0, 1.0),
                "scoreZMm": any(&mut rng, -12.0, 1.0),
                "score": rng.next().is_multiple_of(2),
                "feedMmMin": any(&mut rng, 0.0, 30_000.0),
                "plungeMmMin": any(&mut rng, 0.0, 6000.0),
                "passes": any(&mut rng, 0.0, 30.0),
                "offsetMm": any(&mut rng, 0.0, 3.0),
                "swivelDeg": any(&mut rng, -10.0, 200.0),
                "overcutMm": any(&mut rng, 0.0, 12.0),
            },
        });
        let p = GcodeParams::from_json(&v);
        let g = knife_gcode(&s, &p);
        if g.text.is_empty() {
            assert!(!g.warnings.is_empty() && g.lines == 0, "round {round}: refused without a reason");
            continue;
        }
        play(&g, (p.knife.safe_z_mm * 1000.0).round() / 1000.0);
        assert!(g.burn_mm.is_finite() && g.travel_mm.is_finite() && g.burn_sec.is_finite(), "round {round}");
        let b = g.bounds_mm;
        let (ox, oy, e) = (p.origin_mm.x, p.origin_mm.y, 5.01 + 2.0 * p.knife.offset_mm);
        assert!(b.x0 >= ox - e && b.x1 <= ox + w + e && b.y0 >= oy - e && b.y1 <= oy + h + e, "round {round}: {b:?}");
        assert_eq!(g, knife_gcode(&s, &p), "round {round}: deterministic");
    }
}
