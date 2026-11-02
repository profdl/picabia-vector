//! PIC31.0: the soft-body spike. rapier2d's soft bodies in phase 1's world: determinism (the
//! fingerprint printed here is compared with the wasm32 build's under Node), junk outlines through
//! `polyline`, `polygon` and `volumetric`, step cost and snapshot cost (`#[ignore]`d: run them in
//! release, `cargo test --release -p vectorcraft-physics spike_ -- --ignored --nocapture`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use rapier2d::prelude::*;

include!("spike_scene.rs");

#[test]
fn spike_soft_scene_gives_the_same_bits_twice() {
    let (a, n) = spike_scene(240);
    let (b, _) = spike_scene(240);
    assert_eq!(a, b);
    assert!(n > 500, "{n} words");
    println!("soft spike fingerprint {a:016x} words {n}");
}

/// Junk outlines (metres).
fn junk() -> Vec<(&'static str, Vec<Vector>)> {
    let v = Vector::new;
    vec![
        ("empty", vec![]),
        ("one point", vec![v(1.0, 1.0)]),
        ("two equal points", vec![v(1.0, 1.0), v(1.0, 1.0)]),
        ("collinear", vec![v(0.0, 0.0), v(1.0, 0.0), v(2.0, 0.0), v(3.0, 0.0)]),
        ("tiny", vec![v(0.0, 0.0), v(1e-7, 0.0), v(1e-7, 1e-7), v(0.0, 1e-7)]),
        ("huge", vec![v(-1e6, -1e6), v(1e6, -1e6), v(1e6, 1e6), v(-1e6, 1e6)]),
        ("nan", vec![v(0.0, 0.0), v(f32::NAN, 0.0), v(1.0, 1.0), v(0.0, 1.0)]),
        ("infinite", vec![v(0.0, 0.0), v(f32::INFINITY, 0.0), v(1.0, 1.0)]),
        ("self-crossing", vec![v(0.0, 0.0), v(1.0, 1.0), v(1.0, 0.0), v(0.0, 1.0)]),
        ("zero-length segments", vec![v(0.0, 0.0), v(0.0, 0.0), v(1.0, 0.0), v(1.0, 0.0), v(1.0, 1.0), v(0.0, 1.0), v(0.0, 1.0)]),
        ("closed back on itself", vec![v(0.0, 0.0), v(1.0, 0.0), v(1.0, 1.0), v(0.0, 0.0)]),
        ("spike", vec![v(0.0, 0.0), v(1.0, 0.0), v(1.0, 1.0), v(1.0, -5.0), v(0.0, 1.0)]),
        ("sliver", vec![v(0.0, 0.0), v(1.0, 0.0), v(2.0, 1e-6)]),
    ]
}

/// Cell sizes tried with `volumetric` (metres): sane, junk.
const CELLS: [f32; 6] = [0.1, 0.0, -1.0, f32::NAN, 1e-6, 1e6];

/// Build `b`, insert it on a floor and step 30 times.
fn run_one(b: SoftBodyBuilder) -> usize {
    let mut w = SpikeWorld::new();
    w.walls(10.0, 10.0);
    let h = w.soft.insert(b, &mut w.rbs, &mut w.cols);
    let mut pipeline = PhysicsPipeline::new();
    for _ in 0..30 {
        w.step(&mut pipeline);
    }
    w.soft.get(h).map_or(0, |s| s.num_particles())
}

/// What a builder does with an outline: `Ok(Some(particles))`, `Ok(None)` (refused), or the panic.
fn attempt(f: impl FnOnce() -> Option<SoftBodyBuilder>) -> Result<Option<usize>, String> {
    catch_unwind(AssertUnwindSafe(|| f().map(run_one)))
        .map_err(|e| e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string())).unwrap_or_default())
}

/// The outline as Picabia will hand it to rapier: finite points only, inside ±10 km, repeated
/// points (and a closing repeat) dropped; `None` with fewer than `min` points, an extent under
/// 1 µm, or (closed) no area or crossing edges. Validation we own, as PIC30.2a did for parry's hull.
fn validated(pts: &[Vector], min: usize, closed: bool) -> Option<Vec<Vector>> {
    if !pts.iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 1e4 && p.y.abs() <= 1e4) {
        return None;
    }
    let mut out: Vec<Vector> = pts.to_vec();
    out.dedup();
    if closed && out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    if out.len() < min {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &out {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    if (x1 - x0).max(y1 - y0) < 1e-6 {
        return None;
    }
    if closed {
        let n = out.len();
        let area2: f64 = (0..n)
            .map(|i| {
                let (a, b) = (out[i], out[(i + 1) % n]);
                f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y)
            })
            .sum();
        if area2.abs() < 1e-10 {
            return None;
        }
        // Crossing edges (non-adjacent segments that intersect).
        let seg = |i: usize| (out[i], out[(i + 1) % n]);
        let cross = |o: Vector, a: Vector, b: Vector| f64::from(a.x - o.x) * f64::from(b.y - o.y) - f64::from(a.y - o.y) * f64::from(b.x - o.x);
        for i in 0..n {
            for j in i + 1..n {
                if j == i + 1 || (i == 0 && j == n - 1) {
                    continue;
                }
                let ((a, b), (c, d)) = (seg(i), seg(j));
                let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
                if (d1 * d2 <= 0.0) && (d3 * d4 <= 0.0) {
                    return None;
                }
            }
        }
    }
    Some(out)
}

#[test]
fn spike_junk_sweep() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut raw_panics = Vec::new();
    let mut validated_panics = Vec::new();
    let mut raw_skipped = Vec::new();
    let mut raw_ok = Vec::new();
    for (name, pts) in junk() {
        // Raw: what rapier does on its own.
        let p = pts.clone();
        if let Err(e) = attempt(|| SoftBodyBuilder::polyline(p, None)) {
            raw_panics.push(format!("polyline {name}: {e}"));
        }
        let p = pts.clone();
        if let Err(e) = attempt(|| Some(SoftBodyBuilder::polygon(p))) {
            raw_panics.push(format!("polygon {name}: {e}"));
        }
        for cell in CELLS {
            // rapier fills the outline with no cap on the cell count: a cell tiny next to the
            // outline allocates until the process is killed. Not tried raw (a finding: Picabia
            // caps the count itself).
            let ext = pts.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).fold(0.0f32, |m, p| m.max(p.x.abs()).max(p.y.abs()));
            if cell.is_finite() && cell > 0.0 && ext / cell > 1_000.0 {
                raw_skipped.push(format!("volumetric {name} cell {cell}: ~{:.0e} cells", (ext / cell).powi(2)));
                continue;
            }
            let p = pts.clone();
            match attempt(|| spike_volumetric(&p, cell)) {
                Err(e) => raw_panics.push(format!("volumetric {name} cell {cell}: {e}")),
                Ok(n) => raw_ok.push(format!("volumetric {name} cell {cell}: {n:?}")),
            }
        }
        // Validated first, cell size clamped (0.5% to 50% of the extent, finite).
        let open = validated(&pts, 2, false);
        if let Err(e) = attempt(|| SoftBodyBuilder::polyline(open?, None)) {
            validated_panics.push(format!("polyline {name}: {e}"));
        }
        let closed = validated(&pts, 3, true);
        let c = closed.clone();
        if let Err(e) = attempt(|| Some(SoftBodyBuilder::polygon(c?))) {
            validated_panics.push(format!("polygon {name}: {e}"));
        }
        for cell in CELLS {
            let c = closed.clone();
            if let Err(e) = attempt(|| {
                let c = c?;
                let ext = c.iter().fold(0.0f32, |m, p| m.max((*p - c[0]).length()));
                let cell = if cell.is_finite() { cell.clamp(ext * 0.005, ext * 0.5) } else { ext * 0.1 };
                spike_volumetric(&c, cell)
            }) {
                validated_panics.push(format!("volumetric {name} cell {cell}: {e}"));
            }
        }
    }
    std::panic::set_hook(hook);
    println!("raw rapier panics ({}):\n  {}", raw_panics.len(), raw_panics.join("\n  "));
    println!("raw volumetric not tried ({}):\n  {}", raw_skipped.len(), raw_skipped.join("\n  "));
    println!("raw volumetric results:\n  {}", raw_ok.join("\n  "));
    println!("validated panics ({}):\n  {}", validated_panics.len(), validated_panics.join("\n  "));
    assert!(validated_panics.is_empty(), "{validated_panics:#?}");
}

/// The budget scene: 20 jelly bodies of up to 400 particles and 200 rigid boxes in a walled box.
fn budget_world() -> (SpikeWorld, usize) {
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let mut w = SpikeWorld::new();
    w.walls(24.0, 14.0);
    w.ip.soft_bodies.max_extra_substeps = env("SPIKE_SUBSTEPS", 4.0) as usize;
    w.ip.num_solver_iterations = env("SPIKE_ITER", 4.0) as usize;
    if env("SPIKE_RECOVERY", 1.0) == 0.0 {
        let r = &mut w.ip.soft_bodies.recovery;
        r.inverted_cell_detection = false;
        r.self_crossing_detection = false;
        r.cross_body_detection = false;
    }
    let mut particles = 0;
    let soft = env("SPIKE_SOFT", 20.0) as usize;
    let cloth = env("SPIKE_CLOTH", 0.0) as usize;
    let cell = env("SPIKE_CELL", 0.115);
    let pgs = env("SPIKE_PGS", 3.0) as usize;
    for i in 0..soft {
        let (x, y) = (0.5 + (i % 10) as f32 * 2.3, 1.0 + (i / 10) as f32 * 2.5);
        let b = if i < cloth {
            let n = env("SPIKE_CLOTH_N", 400.0) as usize;
            spike_cloth(Vector::new(x, y), Vector::new(x + 2.0, y), n, 2.0 / n as f32).map(|c| c.self_contacts(env("SPIKE_SELF", 1.0) != 0.0))
        } else {
            spike_volumetric(&spike_rect(x, y, 1.6, 1.6), cell).map(|b| b.material(spike_jelly(10.0)).shape_matching(true).volume_preservation(true))
        };
        if let Some(b) = b {
            let h = w.soft.insert(b.additional_pgs_iterations(pgs), &mut w.rbs, &mut w.cols);
            particles = particles.max(w.soft.get(h).map_or(0, |s| s.num_particles()));
        }
    }
    let rigid = env("SPIKE_RIGID", 200.0) as usize;
    for i in 0..rigid {
        let (x, y) = (0.5 + (i % 40) as f32 * 0.58, 6.5 + (i / 40) as f32 * 0.5);
        w.dynamic_box(x, y, 0.15, 0.12, i as f32 * 0.1);
    }
    (w, particles)
}

#[test]
#[ignore = "timing: run in release"]
fn spike_step_cost() {
    let (mut w, particles) = budget_world();
    println!("soft bodies {} (largest {particles} particles), rigid bodies {}", w.soft.len(), w.rbs.len());
    let mut pipeline = PhysicsPipeline::new();
    let mut worst = 0.0f64;
    let t0 = Instant::now();
    let mut per_second = Vec::new();
    let n: u32 = std::env::var("SPIKE_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    for s in 0..n {
        let t = Instant::now();
        w.step(&mut pipeline);
        worst = worst.max(t.elapsed().as_secs_f64());
        if s % 60 == 59 {
            per_second.push(t0.elapsed().as_secs_f64());
        }
    }
    let total = t0.elapsed().as_secs_f64();
    println!(
        "{n} steps in {total:.3} s: {:.2} ms a step on average, worst {:.2} ms; {:.1} steps a second",
        total * 1e3 / f64::from(n),
        worst * 1e3,
        f64::from(n) / total
    );
    println!("seconds elapsed after each simulated second: {per_second:.2?}");
}

#[test]
#[ignore = "timing: run in release"]
fn spike_snapshot_cost() {
    let (mut w, _) = budget_world();
    let mut pipeline = PhysicsPipeline::new();
    for _ in 0..120 {
        w.step(&mut pipeline);
    }
    let t = Instant::now();
    let snaps: Vec<SpikeWorld> = (0..10).map(|_| w.clone()).collect();
    let clone_ms = t.elapsed().as_secs_f64() * 100.0;
    println!("one snapshot (clone of every set): {clone_ms:.2} ms");
    drop(snaps);
}

#[test]
fn spike_restoring_a_snapshot_replays_the_same_bits() {
    // Run 180 steps straight; then run 90, snapshot, run on, restore, run the last 90 again.
    let (mut straight, _) = small_world();
    let mut p = PhysicsPipeline::new();
    for _ in 0..180 {
        straight.step(&mut p);
    }
    let (mut w, _) = small_world();
    let mut p2 = PhysicsPipeline::new();
    for _ in 0..90 {
        w.step(&mut p2);
    }
    let snap = w.clone();
    for _ in 0..40 {
        w.step(&mut p2);
    }
    // Same pipeline, restored sets.
    let mut a = snap.clone();
    for _ in 0..90 {
        a.step(&mut p2);
    }
    // A fresh pipeline.
    let mut b = snap.clone();
    let mut p3 = PhysicsPipeline::new();
    for _ in 0..90 {
        b.step(&mut p3);
    }
    assert_eq!(straight.bits(), a.bits(), "restored, same pipeline");
    assert_eq!(straight.bits(), b.bits(), "restored, fresh pipeline");
}

fn small_world() -> (SpikeWorld, usize) {
    let mut w = SpikeWorld::new();
    w.walls(6.0, 4.0);
    w.fixed_box(3.0, 2.5, 0.8, 0.1);
    for i in 0..6 {
        w.dynamic_box(0.5 + i as f32 * 0.9, 0.4, 0.2, 0.15, i as f32 * 0.4);
    }
    if let Some(c) = spike_cloth(Vector::new(2.0, 1.5), Vector::new(4.0, 1.5), 40, 0.04) {
        w.soft.insert(c, &mut w.rbs, &mut w.cols);
    }
    if let Some(b) = spike_volumetric(&spike_rect(0.5, 2.8, 0.8, 0.6), 0.12) {
        w.soft.insert(b.material(spike_jelly(6.0)).shape_matching(true).volume_preservation(true), &mut w.rbs, &mut w.cols);
    }
    let n = w.soft.len();
    (w, n)
}
