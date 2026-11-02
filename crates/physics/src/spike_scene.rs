// PIC31.0 spike scenes: plain rapier2d, no Picabia types, so the same file is compiled into the
// native tests (`tests_soft_spike.rs`) and into the wasm32 harness that runs under Node
// (`docs/picabia/physics.md` › Phase 2 › PIC31.0 says how). Included with `include!`, so it holds
// items only.

/// FNV-1a over 64-bit words, as phase 1's fingerprint.
fn spike_fnv(words: impl Iterator<Item = u64>) -> u64 {
    words.fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b).wrapping_mul(0x100_0000_01b3))
}

/// One world, held as phase 1 holds it (the sets, stepped by a `PhysicsPipeline`).
#[derive(Clone)]
struct SpikeWorld {
    rbs: RigidBodySet,
    cols: ColliderSet,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    impulse: ImpulseJointSet,
    multibody: MultibodyJointSet,
    soft: SoftBodySet,
    ccd: CCDSolver,
    ip: IntegrationParameters,
}

impl SpikeWorld {
    fn new() -> Self {
        SpikeWorld {
            rbs: RigidBodySet::new(),
            cols: ColliderSet::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            impulse: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            ip: IntegrationParameters { dt: 1.0 / 60.0, normalized_prediction_distance: 0.2, ..IntegrationParameters::default() },
        }
    }

    fn step(&mut self, pipeline: &mut PhysicsPipeline) {
        pipeline.step(
            Vector::new(0.0, 9.81),
            &self.ip,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.rbs,
            &mut self.cols,
            &mut self.impulse,
            &mut self.multibody,
            &mut self.soft,
            &mut self.ccd,
            &(),
            &(),
        );
    }

    fn fixed_box(&mut self, x: f32, y: f32, hx: f32, hy: f32) {
        let b = self.rbs.insert(RigidBodyBuilder::fixed().translation(Vector::new(x, y)));
        self.cols.insert_with_parent(ColliderBuilder::cuboid(hx, hy).friction(0.5), b, &mut self.rbs);
    }

    fn dynamic_box(&mut self, x: f32, y: f32, hx: f32, hy: f32, angle: f32) -> RigidBodyHandle {
        let b = self.rbs.insert(RigidBodyBuilder::dynamic().translation(Vector::new(x, y)).rotation(angle).ccd_enabled(true));
        self.cols.insert_with_parent(ColliderBuilder::cuboid(hx, hy).restitution(0.35).friction(0.4), b, &mut self.rbs);
        b
    }

    /// A walled box `w` × `h` metres, top-left at the origin (y down).
    fn walls(&mut self, w: f32, h: f32) {
        let t = 0.5;
        self.fixed_box(w / 2.0, h + t, w / 2.0 + t, t);
        self.fixed_box(w / 2.0, -t, w / 2.0 + t, t);
        self.fixed_box(-t, h / 2.0, t, h / 2.0 + t);
        self.fixed_box(w + t, h / 2.0, t, h / 2.0 + t);
    }

    /// Every rigid pose and soft particle position, as bits.
    fn bits(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for (_, rb) in self.rbs.iter() {
            let t = rb.translation();
            out.extend([u64::from(t.x.to_bits()), u64::from(t.y.to_bits()), u64::from(rb.rotation().angle().to_bits())]);
        }
        for (_, sb) in self.soft.iter() {
            for p in sb.particle_positions() {
                out.extend([u64::from(p.x.to_bits()), u64::from(p.y.to_bits())]);
            }
        }
        out
    }
}

/// A closed outline as volumetric input: the points, and segments around them (counter-clockwise
/// in a y-up frame is clockwise on the page; both are tried by the caller's builder).
fn spike_ring(points: &[Vector]) -> (Vec<Vector>, Vec<[u32; 2]>) {
    let n = points.len() as u32;
    (points.to_vec(), (0..n).map(|i| [i, (i + 1) % n]).collect())
}

/// A volumetric soft body from a closed outline, either winding; `None` when rapier makes none.
fn spike_volumetric(points: &[Vector], cell: f32) -> Option<SoftBodyBuilder> {
    let (v, idx) = spike_ring(points);
    SoftBodyBuilder::volumetric(&v, &idx, cell).or_else(|| {
        let mut rev = v.clone();
        rev.reverse();
        let (rv, ridx) = spike_ring(&rev);
        SoftBodyBuilder::volumetric(&rv, &ridx, cell)
    })
}

fn spike_rect(x: f32, y: f32, w: f32, h: f32) -> Vec<Vector> {
    vec![Vector::new(x, y), Vector::new(x + w, y), Vector::new(x + w, y + h), Vector::new(x, y + h)]
}

/// A cloth strip: a polyline of `n` particles from `a` to `b`, low bending, self-contact.
fn spike_cloth(a: Vector, b: Vector, n: usize, thickness: f32) -> Option<SoftBodyBuilder> {
    let pts: Vec<Vector> = (0..n).map(|i| a.lerp(b, i as f32 / (n - 1) as f32)).collect();
    let material = SoftBodyMaterial {
        edge_softness: SpringCoefficients::new(60.0, 1.0),
        bend_softness: SpringCoefficients::new(2.0, 1.0),
        ..SoftBodyMaterial::default()
    };
    Some(SoftBodyBuilder::polyline(pts, None)?.shape_matching(false).self_contacts(true).particle_radius(thickness / 2.0).material(material))
}

fn spike_jelly(material_freq: f32) -> SoftBodyMaterial {
    SoftBodyMaterial {
        edge_softness: SpringCoefficients::new(material_freq, 0.3),
        volume_softness: SpringCoefficients::new(material_freq, 0.3),
        shape_matching_softness: SpringCoefficients::new(material_freq / 2.0, 0.3),
        ..SoftBodyMaterial::default()
    }
}

/// The determinism scene: 8 rigid boxes, 3 cloth strips (one draped over a shelf, one pinned at
/// both ends, one attached to a box), 3 jelly cells bodies, 2 polygon rings; 20 bodies in all
/// (walls and shelf aside). Stepped `steps` times; returns the FNV hash of every body's bits.
fn spike_scene(steps: u32) -> (u64, usize) {
    let mut w = SpikeWorld::new();
    w.walls(8.0, 6.0);
    w.fixed_box(4.0, 3.0, 0.8, 0.1); // the shelf
    let mut carrier = None;
    for i in 0..8 {
        let h = w.dynamic_box(0.6 + i as f32 * 0.9, 0.5 + (i % 3) as f32 * 0.4, 0.2, 0.15, i as f32 * 0.3);
        if i == 6 {
            carrier = Some(h);
        }
    }
    if let Some(c) = spike_cloth(Vector::new(3.0, 2.0), Vector::new(5.0, 2.0), 40, 0.04) {
        w.soft.insert(c, &mut w.rbs, &mut w.cols);
    }
    if let Some(c) = spike_cloth(Vector::new(0.5, 1.0), Vector::new(3.0, 1.0), 50, 0.03) {
        w.soft.insert(c.pinned_particles([0, 49]), &mut w.rbs, &mut w.cols);
    }
    if let Some(c) = spike_cloth(Vector::new(5.5, 1.5), Vector::new(7.5, 1.5), 30, 0.03) {
        let h = w.soft.insert(c, &mut w.rbs, &mut w.cols);
        if let (Some(sb), Some(carrier)) = (w.soft.get_mut(h), carrier) {
            sb.attach_particle(0, carrier, &w.rbs);
        }
    }
    for i in 0..3 {
        let x = 1.0 + i as f32 * 2.2;
        if let Some(b) = spike_volumetric(&spike_rect(x, 3.6, 0.8, 0.6), 0.12) {
            w.soft.insert(b.material(spike_jelly(4.0 + i as f32 * 4.0)).shape_matching(true).volume_preservation(true), &mut w.rbs, &mut w.cols);
        }
    }
    for i in 0..2 {
        let c = Vector::new(2.0 + i as f32 * 3.5, 4.8);
        let pts: Vec<Vector> = (0..16)
            .map(|k| {
                let a = k as f32 / 16.0 * core::f32::consts::TAU;
                c + Vector::new(0.4 * libm_cos(a), 0.4 * libm_sin(a))
            })
            .collect();
        w.soft.insert(SoftBodyBuilder::polygon(pts).material(spike_jelly(8.0)), &mut w.rbs, &mut w.cols);
    }
    let mut pipeline = PhysicsPipeline::new();
    for _ in 0..steps {
        w.step(&mut pipeline);
    }
    let bits = w.bits();
    (spike_fnv(bits.iter().copied()), bits.len())
}

/// Platform-independent sine and cosine for building inputs (not `f32::sin`, whose last bits may
/// differ between the native maths library and wasm's).
fn libm_sin(x: f32) -> f32 {
    // Taylor series about 0 after reducing into [-π, π]: enough for laying out test points.
    let tau = core::f32::consts::TAU;
    let mut x = x % tau;
    if x > core::f32::consts::PI {
        x -= tau;
    }
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))))
}

fn libm_cos(x: f32) -> f32 {
    libm_sin(x + core::f32::consts::FRAC_PI_2)
}
