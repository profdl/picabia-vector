# Physics

**Status:** 2026-11-03, phase 1 (PIC30.1, PIC30.2): drop-and-settle; phase 2 (PIC31.0 to PIC31.8,
soft bodies and live simulation) built and tested headless, not yet tried by hand in the app: its
gate (PIC31.0) failed on cross-platform bits and the owner accepted per-platform soft results (see
[Decisions](#decisions)); the native step budget isn't met (see Budgets). Ported from the owner's
tlprocedural (its Physics modifier: `PhysicsSettings` in `src/types/modifiers.ts`, defaults in
`modifierConfig.ts`, ranges in `modifierSchemas.ts`, the world in
`src/store/modifiers/core/PhysicsEngine.ts` and `PhysicsGeometryConverter.ts`).

Objects get physics settings; **Object ▸ Physics ▸ Simulate** drops them under gravity until they
rest, then moves each object to where it came to rest, as one undo step. VectorCraft documents are
static, so there is no clock and no live playback: nothing moves until Simulate runs.

## How it works

- **Settings** live on the object as a foreign attribute (namespace
  `https://github.com/profdl/picabia-vector/ns/modifiers`, prefix `picabia`, name `picabia.physics`)
  holding JSON, beside the modifier stack. Save and reopen, copy and paste, duplicate and undo carry
  them; plain VectorCraft keeps them and ignores them.
- **Who takes part.** Every visible object whose settings are turned on, and every selected object
  without settings (as a dynamic body with the defaults). An object inside one with settings moves
  with it (a group with settings is one rigid body). Hidden objects are left out. Locked objects (or
  objects on locked layers) stay put as obstacles. Objects whose settings can't be read are skipped
  and reported.
- **The world** is rapier2d 0.36.1 (Apache-2.0), rigid bodies only, with `enhanced-determinism`:
  the same document simulates to the same bits every time on one machine and build. Results may
  differ in the last bits between platforms (macOS arm64 and x86_64 Linux do, measured 2026-11-03;
  rapier calls the platform's `atan2`, see PIC31.0 below), and that is accepted: no test depends
  on them matching. No threads, no `fem`.
- **Steps.** 60 steps per simulated second (tlprocedural's update rate). The run stops when every
  dynamic body sleeps, or at the step cap (default 3,000 = 50 s; 1–20,000). One run may do at most 6
  million body-steps (1,000 bodies get at most 6,000 steps), and at most 1,000 bodies take part
  (dynamic ones first; the rest are listed in `skipped`, before any outline is read). A run has at
  most 1,800 collider pieces (convex parts, polyline segments, boxes, balls); a body that would go
  over collides as its bounding box, with a note. That keeps rapier's contact graph (at most 2²²
  manifolds per bucket) far from full even with every piece piled on one spot. No
  wall-clock limit (the web build has no clock): only step counts, so a run always ends.
- **Moving the objects.** Each dynamic body's motion (a turn about its bounds' centre and a move) is
  applied with the transform commands' own path (`transform_node`, as Transform Each does), so
  paths, live shapes, groups, type, images and symbol instances move and turn as with Rotate and
  Move. Pattern fills follow General › Transform Pattern Tiles (or the `patterns` param).
- **Walls.** The artboard's four edges, 50 pt thick (tlprocedural's paper bounds,
  `createBoundaryWalls`), bounce 0.3 and friction 0.5 (its `DEFAULT_BOUNDS_*`). The app passes the
  active artboard; from scripts the `artboard` param picks one (default: the one under the dynamic
  bodies, else the first). `walls: false` turns them off; a body that then falls more than 1,000,000
  pt from the scene is taken out and left where it was (reported in `lost`).
- **Units.** Points in the document, metres in the world at **100 pt per metre** (an object 100 pt
  across is 1 m), so rapier's tolerances and sleep thresholds suit artboard-sized scenes. Gravity
  defaults to 9.81 m/s² (981 pt/s²) down the page; `gravityX`/`gravityY` set it in pt/s².
- **Modifier stacks.** Phase 1 simulates the object itself: its copies keep their place relative to
  it and move with it. Copies don't collide.

## Settings

| Key | Panel label | Range | Default | What |
|---|---|---|---|---|
| `enabled` | eye | on/off | on | Off: the object takes no part (settings kept) |
| `bodyType` | Body | dynamic, static, kinematic | dynamic | Dynamic falls and collides; static never moves; kinematic stays put too (no clock to move it in phase 1). An unknown type (a later version's, e.g. `soft`) is kept and simulated as static, with a warning |
| `colliderType` | Collider | auto, precise, fast, box | auto | The collision shape (below) |
| `gravityScale` | Gravity | 0–2 | 1.2 | Gravity multiplier (an old `gravity` field is read as this) |
| `restitution` | Bounce | 0–1 | 0.35 | Bounciness; the larger of the two touching values is used |
| `friction` | Friction | 0–1 | 0.4 | The smaller of the two touching values is used |
| `mass` | Extra mass | 0–10,000 kg | 1 | Added to the body's own mass (1 kg/m² of area); 0 = area only |
| `linearDamping` | Drag | 0–20 | 0 | Movement resistance |
| `angularDamping` | Spin drag | 0–20 | 0.02 | Rotation resistance |
| `sensor` | Sensor (passes through) | on/off | off | Feels overlaps but takes no contact forces: passes through everything, walls included |

Parsing is lenient: a number out of range is clamped, a value of the wrong type is ignored (the
default stays), an unknown collider type reads as auto, each with a warning; fields this version
doesn't know are kept on a round trip. Text that isn't a JSON object is reported as damaged
(`physics.get`'s `error`) and the object is skipped by Simulate; `physics.set` replaces it.

## Colliders

Built from the object's geometry in document space (strokes ignored, as the selection box measures
it). Paths are flattened (tolerance 0.2% of the object's diagonal, 0.05–5 pt); a whole live ellipse
within 5% of round is a ball; type, images, symbol instances and other kinds are their bounding
boxes; a group is all its visible members; a clip group is its clipping path.

| Fidelity | Dynamic body | Static body |
|---|---|---|
| box | The object's bounding box | same |
| fast | The convex hull of every outline (each simplified to 32 points first) | same |
| auto | Per closed outline: ≤ 16 points its hull; ≤ 64 the hull of it simplified to 32; more: simplified to 64 and split into convex parts. Open outlines: their line | Same, but more than 64 points: its edges (a cup holds what falls in) |
| precise | Each closed outline split into convex parts (at most 256 points each, simplified beyond) | Each closed outline's edges |

Caps, applied while the outline is read: 100,000 flattened points per object (more: every nth
kept, evenly across all its outlines, so a closed ring still closes on itself), 2,048 outlines read
(the rest left out, with a note); then 2,000 outline points per object (each ring at least 3 and a
share of the rest by size; simplified with Ramer–Douglas–Peucker, as tlprocedural's
`simplifyVertices`, after thinning a long ring evenly to twice its target) and 512 pieces (more: the
object is its convex hull). Geometry that isn't finite, is empty, reaches beyond ±1,000,000 pt or
is smaller than 0.01 pt both ways is skipped with the reason. A body of open outlines only gets the
mass and inertia of its bounding box.

Rings are cleaned (repeated and collinear points dropped) and checked for self-intersection before
they are split: a ring whose edges cross or touch collides as its hull. Every convex collider is
built from its points as the f32 values rapier gets (Picabia computes the hull itself and gives
parry a ready convex polygon): points distinct in f64 can collapse to one f32 point, and parry's
own hull asserts on that, which would abort the web build. Fewer than 3 distinct f32 points, an
extent under 1 µm or no area: a box instead.

## Commands

| Command | Params | Undo |
|---|---|---|
| `physics.set` | `ids?`/`id?`, `props?` (the keys above) | one step ("Physics Settings"); none when nothing changes |
| `physics.clear` | `ids?`/`id?` | one step ("Clear Physics") |
| `physics.get` | `id?` → `{id, settings, warnings, error?}` | query |
| `physics.simulate` | `steps?`, `stopWhenSettled?`, `gravityX?`, `gravityY?`, `walls?`, `artboard?` → `{bodies, moved, steps, seconds, settled, lost, skipped, warnings, notes, artboard}` | one step ("Simulate Physics"); none when nothing moved |

UI: Object ▸ Physics ▸ Simulate, Add Physics, Make Static, Settings… (`ui.physicsSettings`: adds
settings where there are none and opens the Appearance panel's physics row), Clear Physics. The
Appearance panel shows a "Physics: Dynamic" row under the modifier rows when a selected object has
settings: the eye, the fields, Simulate and Clear Physics.

## Deliberate differences from tlprocedural

- **No live simulation.** tlprocedural plays the world every frame and shows overlay copies; here
  Simulate runs to rest at once and moves the real objects, one undo step.
- **Scale and gravity.** tlprocedural runs rapier in pixels (1 px = 1 world unit) with gravity 90
  px/s², a slow, floaty fall; Picabia uses 100 pt per metre and 9.81 m/s², so falls look physical
  and settle in fewer steps. The settled result is what matters without a clock.
- **Contact margin.** A 20 pt prediction distance (rapier's default is 2 pt at this scale): without
  it a dropped box sank 12 pt into the floor in one step and kicked out spinning. Rapier's block
  solver is on.
- **Exact colliders.** tlprocedural's precise and complex auto shapes are triangle meshes. A triangle
  mesh has no volume, so dynamic bodies here split the outline into convex parts instead (ear
  clipping, merged while convex; written for Picabia, not rapier's V-HACD, whose voxeliser can panic
  on degenerate input). Static bodies collide with the outline's edges.
- **Ellipses and circles.** A ball only for a near-round live ellipse; other ellipses are their
  polygon hull (tlprocedural used a 32-point hull too).
- **Mass.** As tlprocedural, `mass` is added to the area mass (rapier's `additional_mass`), with
  density 1 kg/m² at this scale.
- **Global collider quality** (tlprocedural's per-session override) and **custom bounds** sizes are
  not ported: per-object `colliderType`, and the artboard as the bounds.
- **Not ported:** spawners, throwing, linkages (joints), the physics playback controller (phase 2
  plans its own, below).

## Phase 2: soft bodies and live simulation (planned)

Planned 2026-11-02 with the owner; built 2026-11-02/03 (each part's **As built** note says what
changed from the plan). Any object (or several) can be given a soft-body
preset, fine-tuned, and simulated live on the canvas alongside the rigid bodies: a cloth strip drapes
over a static shelf, a jelly lands on a box and wobbles. The canvas draws simulated clones; the real
objects change only on **Apply**.

### Decisions

| Question | Decision |
|---|---|
| What "cloth" means in 2D | **Side view**: a flexible strip seen edge-on, which falls, drapes over other bodies and can fold onto itself (rapier's own cloth is 3D only) |
| How the result reaches the art | As with modifiers: the simulation shows **clones**; **Apply** warps (soft) or moves (rigid) the original art to the shown pose, one undo step. The art keeps its fills, strokes and gradients |
| Live or settle | **Live** in the first release: play, pause, reset, **scrub** the timeline, **poke** bodies with the mouse while playing, and **settings take effect while playing** |
| First presets | **Cloth** and **Jelly / Rubber** (one elastic model, two presets: soft and wobbly, stiff and bouncy), each fine-tuned in the panel |
| What bodies collide with | **Only objects with physics settings** (static, dynamic, kinematic, soft), as in phase 1. To drape over a drawing, give it Make Static |
| Holding cloth up | **Pin chosen points** (fixed in place) and **attach points to an object** (they move with that body) |
| Cross-platform bits for soft bodies (2026-11-02, after PIC31.0) | **Given up for soft bodies** (option 3 below). Picabia ships for the desktop; Apply bakes the result into the art, and scrubbing and replay happen on one machine, so a pose that differs slightly between platforms costs little. Soft bodies give the same bits on one machine and build (replay, scrubbing, snapshots: tested) and may differ between platforms and in wasm32. Rigid-only scenes don't match across platforms either (measured 2026-11-03, PIC31.9: macOS arm64 and x86_64 Linux differ in the last bits, which phase 1's claim missed). **Owner, 2026-11-03: results need not match across platforms**, and no test may depend on it; the fingerprint is printed, not pinned. rapier isn't patched or vendored; cross-platform bits can come back when a rapier release carries the fix below (re-run PIC31.0's hash check then) |
| Rigid physics | **One flow**: rigid and soft bodies share one world, one transport bar, clones and Apply. Today's instant Simulate becomes **Simulate to Rest** (run to rest, show it, Apply); `physics.simulate` keeps working for scripts (runs to rest and applies, as now) |

### The engine

rapier2d 0.36.1, already a dependency, has soft bodies in its default build (only its separate FEM
solver is behind the `fem` feature, which stays off): `SoftBodyBuilder::polyline`/`rope` (a strip or
line), `polygon`/`volumetric` (a filled 2D body, `volumetric` needs `std`, which wasm32 has),
`SoftBodyMaterial` (stiffness and damping per constraint family, shape matching, volume preservation,
plasticity, tearing), `pinned_particles`, `attach_particle` (a particle held to a rigid body), contact
with rigid colliders and self-contact. Picabia writes no solver: it builds the bodies from document
geometry, maps the result back to the art, and runs the clock.

**Risk, checked first (PIC31.0):** the soft-body module must keep phase 1's guarantees: same bits
natively and in wasm32 with `enhanced-determinism`, no panic on junk input (degenerate, tiny, huge,
self-crossing outlines), and a step cost that allows live playback. If any fails, the plan is
revisited before more is built.

### PIC31.0 result: no-go (2026-11-02), overruled by the owner

The spike failed condition 1. The owner accepted that soft bodies may differ between platforms
(Decisions, above), so the work goes on from PIC31.1. The spike's code is in
`crates/physics/src/spike_scene.rs` (plain rapier, no Picabia types) and `tests_soft_spike.rs`.

| Condition | Result | Numbers |
|---|---|---|
| 1. Same bits natively and in wasm32 | **Fails** | A 20-body scene (8 boxes; 3 cloth strips: draped, pinned at both ends, attached to a box; 3 jelly cell bodies; 2 polygon rings) hashed with phase 1's FNV over every rigid pose and particle position: the same at steps 0, 1, 2, 5, 10 and 30, then different (240 steps: native `4c5b3c80604096f9`, wasm32 under Node `4c67464e3567face`). Same run twice natively: same bits |
| 2. No panic on junk outlines | Passes with our validation | Raw rapier panics on `polygon` with fewer than 3 points (index out of bounds) and on a NaN point (`polyline` and `polygon`: "loosening margin must be non-negative"), and `volumetric` has no cap on the cell count (a 1 µm cell in a 1 m outline allocates until the process is killed). Validated first (finite, inside ±10 km, repeats dropped, ≥ 2 or ≥ 3 points, extent ≥ 1 µm, closed: some area and no crossing edges; cell size clamped to 0.5–50% of the extent): 13 junk outlines × `polyline`, `polygon`, `volumetric` at 6 cell sizes, built, inserted and stepped 30 times: no panic |
| 3. Step cost: 20 soft bodies of ≤ 400 particles and 200 rigid at 60 steps a second natively | Fails with rapier's defaults; jelly passes natively when tuned, cloth only without self-contact | Measured on the 4-core Linux container, idle, release build; 20 jelly bodies of 406 particles each (`volumetric`, shape matching, volume preservation) and 200 boxes, 300 steps. Defaults: 28.1 ms a step (35.6 steps a second), worst 77 ms. Extra PGS iterations 0 (rapier's default 3): 19.0 ms. Also extra substeps 0: 17.3 ms (57.7 steps a second), worst 46 ms. Rigid bodies alone: 0.16 ms. wasm32 under Node (same scene, 240 steps): 40.9 ms a step with defaults, 24.6 ms with PGS 0 (40.6 steps a second; the target is at least 30) |
| 4. Snapshots for scrubbing | Passes | Every set (bodies, colliders, islands, broad and narrow phase, joints, soft bodies, CCD) is `Clone`. One snapshot of the budget scene (a clone of every set) takes 5.3 ms, about one step's cost: a snapshot a second adds under 1% to playback. Restoring a clone and stepping on gives the same bits as never stopping, with the same pipeline or a fresh one |

**How the wasm check runs.** A scratch crate outside the workspace, pinned to the workspace's
versions of rapier2d, parry2d, nalgebra, simba, glam/glamx, libm and wide, `include!`s
`spike_scene.rs`, exports `fingerprint(steps)` from a `cdylib`, and is built with `cargo build
--release --target wasm32-unknown-unknown`. Node instantiates the `.wasm` with no imports and prints
the hash. The same crate built natively gives the workspace test's hash.

**Why condition 1 fails.** rapier2d 0.36.1 computes each soft body's cluster frame (and shape
matching, and corotational cells) with `extract_rotation`
(`src/dynamics/soft_body/soft_body_shape_matching.rs`, line 23), which calls `f32::atan2` from std.
`enhanced-determinism` routes simba's and glam's maths through libm, but this call is on the
primitive: natively it is the platform's C library (glibc here; macOS and Windows have their own),
in wasm32 the Rust libm port. They differ in the last bit now and then, and the simulation
amplifies it. Every soft body takes this path every step, so no setting avoids it, and validating
input can't either. Checked: a scratch copy of rapier with only that call changed to simba's libm
`atan2` gives the same bits natively and in wasm32 at every checkpoint (`4c67464e3567face` at 240
steps). That copy was a diagnostic only and is not in the repo. rapier2d 0.36.1 is the newest
release (crates.io, 2026-11-02). Rigid bodies alone still match (phase 1's check).

The same cause also breaks native-to-native equality between platforms (glibc, Apple's libm and
Microsoft's CRT differ), which phase 1 promises.

**Options (the owner's call):**

1. **Fix upstream.** Report it to dimforge/rapier (a one-line change: `extract_rotation` through
   simba's `RealField::atan2`, as the rest of the crate does under `enhanced-determinism`; check the
   other std calls too: `acos` in 3D bending, `powf` in plasticity and the corotational cell size,
   `exp` in deformation damping and tear smoothing). Wait for the release, then run PIC31.0 again.
   Cleanest; the timing is out of our hands.
2. **Patch rapier in Picabia.** `[patch.crates-io]` pointing at a copy of rapier2d 0.36.1 with the
   fix (a vendored crate of about 80,000 lines of Rust, plus a seam in the workspace `Cargo.toml`). Works now;
   every rapier upgrade has to carry the patch until upstream takes it.
3. **Give up cross-platform bits for soft bodies.** Same bits on one machine and build (scrubbing and
   replay still work, they replay on the same machine), but a document may simulate differently on
   another platform or on the web. Rigid-only scenes keep phase 1's guarantee. Cheapest; weakens a
   promise the plan made.

Recommendation: 1 and 2 together. Patch now (option 2, recorded in `SEAMS.md` as a temporary
seam), file the upstream fix, and drop the patch when a release carries it.

**Upstream bug report** (text for the owner to file at dimforge/rapier; not posted):

> **2D soft bodies are not deterministic across platforms with `enhanced-determinism`**
>
> rapier2d 0.36.1, features `dim2 f32 std block-solver enhanced-determinism`. A scene of soft
> bodies (`polyline`, `polygon`, `volumetric`) and rigid bodies, stepped at 1/60 s, hashed over every
> rigid pose and particle position (FNV-1a of the f32 bits): native x86_64 Linux and
> wasm32-unknown-unknown (Node 22) agree for 30 steps, then differ; after 240 steps
> `4c5b3c80604096f9` (native) vs `4c67464e3567face` (wasm32). Rigid-only scenes agree.
>
> Cause: `extract_rotation` (`src/dynamics/soft_body/soft_body_shape_matching.rs`, line 23, 2D
> branch) calls `sin.atan2(cos)` on the `f32` primitive, which is the platform's libm (glibc,
> Apple's, the MSVC CRT, or Rust's libm port in wasm32), not simba's `RealField::atan2`, which
> `enhanced-determinism` routes through `libm`. It runs every step for every soft body (cluster
> frames, shape matching, corotational cells). Fix, one line:
> `Rotation::new(<Real as simba::scalar::RealField>::atan2(sin, cos))`; with it, the native and
> wasm32 hashes match at every checkpoint (`4c67464e3567face` at 240 steps).
>
> Other std calls on primitives in the same code paths, worth routing through simba too: `acos` in
> `soft_scalar_constraint.rs` (3D dihedral bending) and `soft_body_builder_mesh_helpers.rs`;
> `powf` in `soft_constraint_plasticity.rs` and `soft_element_constraint_assembly.rs` (the
> corotational cell size); `exp` in `soft_element_constraint_assembly.rs` (deformation damping) and
> `soft_body_material.rs` (tear smoothing); `sin`/`cos` in the `disk` and 3D builders (input
> only).

**Cost findings for the next attempt:** cloth is cheap without self-contact (20 strips of 400
particles and 200 boxes: 6.9 ms a step; of 100 particles: 1.9 ms) and very expensive with it (one
400-particle strip alone: 62 ms a step; 20 strips of 100: 68 ms). Self-collide should default to off
for Cloth, or be limited to short strips, and the budget scene should set the soft bodies' extra
PGS iterations and extra substeps to 0 (or tune them per preset) before PIC31.2 tunes the presets.
The jelly scene is at the edge of the native budget even then (worst steps 2.7 frames).

### Bodies from objects

- **Cloth.** The object's long axis (of its bounds, turned with it) is the strip, its short side the
  thickness. The strip is a line of particles along the centre (`polyline`), with particle radius half
  the thickness, low bending stiffness and self-contact. Any shape can be cloth: it bends along that
  axis.
- **Jelly / Rubber.** The object's outlines (flattened as phase 1 does, resampled to the preset's
  spacing) filled with cells (`volumetric`), with shape matching so it springs back to its own shape
  and volume preservation so it doesn't collapse. Holes in compound paths stay holes.
- A group with soft settings is one soft body (its members' outlines together).
- Caps as phase 1: particles per object and per run, cells per object, a body that would go over
  gets a coarser mesh, with a note; untrusted geometry skipped with the reason.
- **As built (PIC31.2):** `mesh.rs`. Cloth's axis is the outline points' principal axis (a nearly
  round outline: its bounds' wider side), pointing left to right on the page; the strip is the
  points' extent along it, its thickness their extent across (at least 0.5 pt), particles
  `resolution` apart. Jelly and rubber: closed outlines cleaned (repeats and straight-line points
  dropped), a crossing one filled as its hull (with a note), resampled to `resolution`, filled by
  rapier's `volumetric` (parry's Delaunay refinement); a hole is an outline inside another of the
  same path or compound path (even-odd nesting; a group's members never make holes in each other);
  separate outlines are pieces of one body (at most 64; outlines smaller than a cell left out).
  Outlines that overlap across group members, or cross within one path, are filled together as
  their hull (with a note). Resampling keeps every corner, splitting each edge into pieces at most
  `resolution` long. Cloth's axis weighs the outline's edges by length, so a finely flattened curve
  doesn't turn it. Caps: 1,200 particles and 2,400 cells per
  body, 12,000 particles per run (the caller's budget): the spacing grows (estimated from the
  outline's area and length before meshing, checked after, up to 8 tries) with a note. Open
  outlines alone can't be jelly (refused with the reason). Never panics: rapier sees only finite,
  simple rings of 3+ points with area and a capped cell count.

### Clones and Apply

- While a simulation session is open, the canvas draws each body's clone at the current moment; the
  originals are dimmed. Clones aren't selectable and aren't exported or saved.
- **Rigid clone:** the art moved and turned (phase 1's transform). **Cloth clone:** the art bent
  along the strip's simulated centre line (each point placed by its distance along the axis and its
  offset across it). **Jelly clone:** the art carried through the simulated cells (each point by its
  position in its rest cell), with curves subdivided so straight sides bend smoothly
  (`vectorcraft_doc::live::map_nonlinear_within`, as Deform uses).
- **As built (PIC31.3):** `pose.rs`. Cloth: a point's distance along the rest axis finds its place
  on the simulated centre line (beyond the ends the end segment is extended), its offset across is
  kept along the line's normal, which turns smoothly between particles. Jelly: a uniform grid of the
  rest cells finds the cell under a point (barycentric); a point outside every cell (the art's
  edge between resampled points) follows the nearest cell's affine. Paths are split into pieces of
  half the particle spacing (an object's paths share 50,000 new anchors by their segment counts);
  placed gradients follow the warp where they sit, opacity masks bend with the art, gradient meshes
  bend point by point; type, images, symbol instances and other kinds take the warp's best-fit
  affine at their centre (their gradients and masks with them). A point outside the mesh follows
  the cell whose centre is nearest, searched ring by ring outwards (a degenerate cell passed over).
  Apply bakes each object once (the first pose given for it), never a layer, and not an object
  inside another one being baked (it moves with that one). Apply
  (engine `picabia_physics_apply.rs`) bakes rigid motions with `transform_node` and replaces soft
  bodies' objects with their warped art in one edit; phase 1's Simulate now bakes through it too.
- **Apply** bakes the shown pose into every body in the session and closes it (one undo step,
  "Apply Physics"). **Cancel** closes it and leaves the art untouched. A live rectangle or ellipse a
  soft body bends becomes a path; a rigid one stays live.

- **As built (PIC31.5):** the canvas draws a *display document* while a session is open: the
  document at rest with each moved body's clone (its pose: rigid moved and turned, soft bent)
  inserted above its object, which is drawn at a quarter of its opacity. Clones have their own
  ids in that copy only; it is never saved, exported, selected or hit-tested (one canvas seam swaps
  the document the renderer draws). A body that hasn't moved shows as itself. Deviation: the
  selection outline and handles stay on the objects at rest.

### The clock and the transport bar

- A slim transport bar (the "slim bar now, keyframe timeline later" of `plan/reference/decisions.md`):
  play/pause, reset, a scrubber with the time, Simulate to Rest, Apply and Cancel.
- **As built (PIC31.5):** the bar floats at the bottom of the canvas while a session is open in
  the active document: play/pause and back-to-start (glyphs drawn in code), the scrubber (0 to the
  furthest step reached, at least a second), the time, To Rest, Apply and Cancel; every button is
  a `physics.*` command. Object ▸ Physics: Simulate to Rest (`physics.toRest`), Start Simulation
  (`physics.play`), Drop and Apply (phase 1's instant `physics.simulate`), Add Physics, Make
  Static, Make Cloth / Jelly / Rubber, Settings…, Clear Physics. The physics row's Simulate button
  runs Simulate to Rest. The app steps a playing session every frame (`session::tick`, at most 4
  steps a frame) and keeps repainting while it plays. PIC31.5a (review): the display document is
  drawn only while it was built from the document as it is (after an edit the document itself
  shows until the session follows on the next frame, which redraws); clones move pattern fills as
  Apply does (Transform Pattern Tiles); the four session-opening commands get the active artboard
  as Simulate does; the bar sits at the canvas's bottom centre, as wide as fits (on a narrow
  canvas the time and To Rest make room; the scrubber's tooltip shows the time). Gaps: every
  step rebuilds the display document and moves the document's revision, so caches keyed by it
  rebuild at the frame rate while playing; dragging the scrubber seeks every frame (backwards from
  the nearest snapshot).
- 60 steps per second, as phase 1. A frame is drawn with the newest finished step, so a slow scene
  plays slower than real time but stays exact.
- **Scrub:** the session keeps a world snapshot every second in memory (capped; older ones thinned).
  Seeking restores the nearest earlier snapshot and steps forward.
- **Pokes and setting changes are recorded** at their step, so scrubbing back and forth replays the
  same motion. Playing or poking after scrubbing back drops what was recorded after that moment.
- **Poke:** while the bar is open, dragging on a clone with the Selection tool pulls it by a spring
  (a soft body by its nearest particle, a rigid body by the grabbed point); letting go throws it.
- **As built (PIC31.6):** `picabia/sim_poke.rs`, reached through one canvas seam before the
  tool's input. While a session is open and the Selection tool is current, a press on a moving
  body where it is shown now (`session::body_at`: a rigid body's bounds moved with it, a soft body's
  particles' bounds grown by their radius) grabs it there and plays; each frame of the drag records
  a pull to the pointer (a critically damped 4 Hz spring; a soft body by its nearest particle with
  the geometric mean of its own mass and the body's); the release lets go, so the body is thrown.
  A press anywhere else reaches the tool. `physics.poke` does the same for scripts. Deviation: the
  hit test is by bounds, not the exact outline. PIC31.6a (review): only a plain primary press
  grabs (not another button, a modifier or Space held); a pull is sent only when the pointer moves;
  the held body is let go when the session closes or the tool changes mid-drag; a release is never
  refused (not even with a full log); `physics.poke` and `physics.seek` aren't journaled (they run
  every frame of a drag or a scrub; the session keeps its own log).
- **Settings while playing:** a change in the physics row is applied to the running world at the
  next step (material, gravity, bounce, friction, damping). A change that needs a new mesh
  (resolution, cloth to cell body or back, self-collide, pins) restarts that body from its rest
  pose, with a note.

- **As built (PIC31.7):** `physics.set` on a body of an open session (the physics row, a script)
  records the change at the current step: bounce, friction, sensor, a rigid body's gravity and drag,
  and a soft body's stiffness, bend, shape memory and damping retune the running body; another kind
  of body, collider, extra mass, or a soft body's preset kind (cloth to cells), detail,
  self-collide, pins, gravity or drag restart that body alone from its rest pose, with a note
  (rapier keeps a soft body's gravity and drag per particle from when it was made). Scrubbing back
  before the change replays the old settings. Turning a body's settings on or off changes who
  takes part: the session starts again. The physics row: Body has Soft; a soft body's row reads
  "Physics: Soft (Cloth)" and shows Preset, Stiffness, Bend (cloth) or Shape memory (jelly,
  rubber), Wobble damping, Detail and Self-collide (the preset's values unless set; no Collider).
  PIC31.7a (review): the world keeps what each body is now (mesh, settings, pins), and snapshots
  keep theirs, so scrubbing back before a restart shows the body with its old mesh (and its pins'
  markers); when the event log is full, a settings change starts the session again instead of
  being dropped. Undoing a settings change restarts the session from step 0 (an edit).

### Settings

`bodyType: "soft"` (phase 1 already keeps unknown body types) with a `soft` object: `preset`
(`cloth`, `jelly`, `rubber`) and only the fields changed from it, so a later tuning of a preset
reaches objects that didn't override it. Fields (starting points, tuned in PIC31.2):

| Key | Label | Cloth | Jelly | Rubber | What |
|---|---|---|---|---|---|
| `stiffness` | Stiffness | high | low | high | Resistance to stretching (natural frequency) |
| `bend` | Bend | low | — | — | Resistance to bending (cloth) |
| `shapeKeep` | Shape memory | — | medium | high | Pull back to its own shape (jelly, rubber) |
| `damping` | Wobble damping | medium | low | medium | How quickly wobble dies out |
| `resolution` | Detail | — | — | — | Particle spacing in pt; finer is smoother and slower |
| `selfCollide` | Self-collide | on | off | off | Folds rest on themselves |
| `pins` | — | none | none | none | Pinned points (below) |

**As built (PIC31.1):** the fields are 0–1 amounts except `resolution` (2–100 pt) and
`selfCollide`; values per preset:

| Key | Cloth | Jelly | Rubber | rapier mapping |
|---|---|---|---|---|
| `stiffness` | 0.85 | 0.3 | 0.85 | edge and area springs, 1–60 Hz |
| `bend` | 0.1 | — | — | bending edges, 0.2–20 Hz |
| `shapeKeep` | — | 0.4 | 0.85 | shape matching, off at 0, else 0.5–20 Hz |
| `damping` | 0.3 | 0.1 | 0.45 | damping ratio 0.05–1 of every spring (not squared; the three above are squared first, so the low end is finely graded) |
| `resolution` | 6 pt | 12 pt | 12 pt | particle spacing |
| `selfCollide` | off | off | off | self-contact (PIC31.0: costly, so off by default for cloth too) |

A self-colliding cloth's particles are at least 1.25 particle radii (half its thickness) apart,
coarser than `resolution` when the strip is thick, with a note: rapier without its FEM solver
spares only a particle's direct neighbours from self-contact, so particles closer than their skin
pushed each other apart for ever (the strip stretched and stood stiff instead of falling).

`null` for a field goes back to the preset's value; a change of `preset` keeps the overrides; an
unknown preset (a later version's) is kept and simulated as jelly. Pins are stored as `u`, `v`:
fractions of the object's geometric bounds (0,0 top left, 1,1 bottom right; ±4 at most), so they
follow the object when it moves, scales or is edited, plus `attachTo` (an object id); at most 64
(so the largest settings stay well within the 16 KB the settings may take); a pin's unknown fields
are kept.
Soft settings are kept when the body type changes to rigid and back.

**Tuned (PIC31.2b)**, measured in a running world (`tests_presets.rs`): a 100 × 80 ellipse dropped
on the floor: jelly settles in about 330 steps (5.5 s), squashes to 0.77 of its height on landing
and keeps 93% of its area at rest; rubber settles in about 610 steps, squashes to 0.83, keeps 99%;
both rebound about 65 pt. Cloth pinned at both ends of a 300 pt strip sags about 58 pt and
stretches about 11% under its own weight. Cloth stiffness peaks at 0.85: at 1.0 (60 Hz, past what
the solver's substeps hold) the strip stretched 29%, at 0.5 80%; more damping stretched it more
(0.3 is the least). Gap: rapier's particle chains stretch about a tenth under their weight; a
stiffer cloth needs more solver iterations (a cost PIC31.0 measured). Phase 1's Simulate (instant
drop and settle) leaves soft bodies in place; the live session (PIC31.4) moves them.

The phase 1 fields still apply (gravity scale, bounce, friction, extra mass, sensor). Pins are stored
as places on the object (fractions of its bounds, above) plus an optional object id to attach to,
not as anchor indices, so they survive editing the path; at build time each pin takes the nearest
particle. Pinning: Direct
Selection on anchors, then Object ▸ Physics ▸ Pin Points; Attach Points To… picks the object (it
must have physics settings; a rigid body carries the pinned points with it).

### Commands

**As built (PIC31.4)** (`picabia_physics_session.rs`; the world and clock in `vectorcraft_physics`
`world.rs` and `session.rs`):

| Command | Params | What |
|---|---|---|
| `physics.session.open` | `gravityX?`, `gravityY?`, `walls?`, `artboard?` (as Simulate) | Opens a live session in the active document at step 0, paused (replaces one already open) → bodies, soft, skipped, warnings, notes, artboard |
| `physics.session.close`, `physics.cancel` | — | Closes it; the art is untouched |
| `physics.session.info` | — | `{open, step, seconds, playing, settled, bodies: [{id, kind}], events, notes, skipped, warnings, artboard}` (query) |
| `physics.play`, `physics.pause` | Play: `gravityX?`, `gravityY?`, `walls?`, `artboard?` (as `physics.session.open`, when it opens one) | Play opens a session when none is; the app steps it each frame (at most 4 steps a frame) |
| `physics.reset` | — | Step 0, paused; recorded pokes kept |
| `physics.seek` | `step` (0–36,000) | Back from the nearest snapshot, forward by stepping |
| `physics.toRest` | `steps?` (default 3,000); `gravityX?`, `gravityY?`, `walls?`, `artboard?` (as `physics.session.open`, when it opens one) | Simulate to Rest: runs until every body sleeps (or the cap), opening a session when none is; the result shows until Apply or Cancel |
| `physics.poke` | `id` + `point` (grab), `to?` (pull), `release?` | Records a poke at the current step |
| `physics.apply` | `patterns?` | Bakes every body's shown pose, one undo step ("Apply Physics"), and closes the session |

The session is engine state, not the document: nothing changes until Apply, and it belongs to the
document it was opened in (another document's commands don't see it). When that document changes
(an edit, an undo) the session starts again from the new rest pose, with a note. `physics.simulate`
keeps its params and output: a scene with soft bodies runs through the live world to rest and bakes
in one step; a rigid-only scene runs phase 1's drop unchanged. Snapshots: every 60 steps, at most
64 (then every other one is dropped and the interval doubles); events at most 20,000 (consecutive
pulls in one step keep the last); a snapshot is the world before its step's events, so replaying
applies each once. PIC31.4a (review): one command steps at most phase 1's 6 million body-steps
(a rigid body counts one, twenty soft particles one; `seek` and `toRest` answer `capped`); soft
bodies fall out of the world as rigid ones do (left where they were, reported); a new grab lets go
of the last; a grab and a pull can be recorded at the same step (paused, or a script); restarting
a body gives its collider pieces back and re-attaches the pins held to it; a change that isn't an
edit (type laid out again) doesn't restart the session, and a restart keeps the objects that took
part without settings; a failed Apply keeps the session; soft cells weigh by their own area.

**As built (PIC31.8)** (`picabia_physics_pins.rs`): `physics.pin` (`id` + `points`, or the anchors
chosen with Direct Selection on soft objects (a path inside a soft group pins the group);
`attachTo`, or `attachToSelected`: the other selected object, which must have physics settings and
not be soft) and `physics.unpin` (pins within 4 pt of each point, or of the chosen anchors; with
none chosen, every pin of the selected soft objects), each one undo step ("Pin Points", "Unpin
Points"); an open simulation restarts the changed bodies with their pins (PIC31.7's path). Object ▸
Physics: Pin Points, Attach Points to Selected, Unpin Points. The canvas marks pins (a ring pinned
in place, a dot attached) on the selected soft objects, and on every body where it is now while a
simulation is open. A pin stores a place on the object (its bounds), and at build time takes the
nearest particle; a pin attached to an object that isn't a rigid body in the simulation holds in
place, with a warning. PIC31.8a (review): pins go on the body the simulation takes (from the
outside in, the first object with settings turned on; a soft path inside a rigid group can't be
pinned), places are measured as the simulation measures bounds (symbol instances by their art),
pins past 64 are reported, pinning what is already pinned makes no undo step and keeps its
attachment unless one is given, Attach Points to Selected takes the first other selected object
with rigid settings, and markers of a soft group show once. The commands are enabled with a
document (`id` + `points` need no selection); with nothing chosen they say what to select.

### Commands (planned)

`physics.session.open/close`, `physics.play`, `physics.pause`, `physics.reset`, `physics.seek`
(`step`), `physics.poke` (`id`, `point`, `to`, scripts and tests), `physics.apply`,
`physics.cancel`, `physics.toRest`, `physics.pin` / `physics.unpin` (`id`, `points`, `attachTo?`).
`physics.set/clear/get/simulate` stay. The transport bar and canvas drawing are UI; everything they
do goes through these commands.

### Tasks

| Id | What |
|---|---|
| PIC31.0 | Spike: soft bodies in the phase 1 world. Determinism hash native vs wasm32, junk sweep of the builders, step cost for 20 soft and 200 rigid bodies, snapshot cost. Go/no-go |
| PIC31.1 | Soft settings: `bodyType: "soft"`, presets and overrides, pins; lenient parse, round trip. **Done** (2026-11-02): `soft.rs`; interfaces stubbed for PIC31.2–31.4 (`mesh.rs`, `pose.rs`, `world.rs`, `session.rs`) |
| PIC31.2 | Bodies from objects: cloth strip and jelly/rubber cells, caps, junk; tune the presets. **Meshing done** (2026-11-02); the presets are tuned once the world runs (PIC31.2b) |
| PIC31.3 | Pose to art: rigid, cloth and jelly warps; Apply as one undo step. **Done** (2026-11-02): warps and the bake; the `physics.apply` command comes with the session (PIC31.4) |
| PIC31.4 | Session and clock: world, steps, snapshots, event log, `physics.session/play/pause/reset/seek/apply/cancel`, rest detection. **Done** (2026-11-02): commands above |
| PIC31.5 | Transport bar and clones on the canvas; Simulate to Rest; rigid bodies on the same flow. **Done** (2026-11-02), checked headless only |
| PIC31.6 | Poke (Selection tool drag while the session is open) and `physics.poke`. **Done** (2026-11-02), checked headless only |
| PIC31.7 | Settings while playing (applied next step; re-mesh restarts the body). **Done** (2026-11-02), the row checked headless only |
| PIC31.8 | Pins and attachments: Pin Points, Attach Points To…, canvas markers. **Done** (2026-11-02), the markers checked headless only |

Budgets (targets, to be measured in PIC31.0 and kept by `vectorcraft-cli perf`): live at 60 steps a
second natively with 20 soft bodies of up to 400 particles and 200 rigid bodies; the web build at
least half that.

**Measured (PIC31.4b, 2026-11-03)** with the real pipeline (outline → mesh → world;
`tests_world.rs` `world_step_cost`, release, this 4-core container, load about 1): 20 jelly ellipses
of up to 379 particles and 200 boxes take 20.6–21.9 ms a step (46–48 steps a second; worst step
40 ms), so **the native budget is not met**: such a scene plays at about 0.8× real time (the clock
takes at most 4 steps a frame, so the app stays responsive and the motion stays exact). A world
snapshot costs 6.6–7.8 ms. The web build wasn't measured with the real pipeline (PIC31.0's spike:
24.6 ms a step in wasm32 under Node for its similar scene, above half the native target). Not added
to `vectorcraft-cli perf` (an upstream file); left as a follow-up.

### Open questions

- Is the session saved with the file (settings, pins, recorded pokes and the current time), or is it
  a working state that closes on save? Proposed: not saved; Apply or Cancel before saving, and a
  save with an open session asks.
- Does Apply bake every body in the session, or can it bake only selected ones and keep the rest
  running? Proposed: every body first; Apply Selected later.

- Decided alone during PIC31.4 (2026-11-02): (1) **Saving while a session is open** saves the art
  at rest without asking: the session never changes the document, so there is nothing to lose
  (the open question's proposed prompt isn't needed; the session stays open). (2) **Apply** bakes
  every body (the proposed answer; Apply Selected later). (3) The session belongs to one document;
  editing that document restarts it from the new rest pose rather than closing it. (4) One session
  at a time: opening one replaces the last. (5) `physics.simulate` keeps phase 1's drop for
  rigid-only scenes (its results unchanged, bit for bit) and uses the live world when soft bodies
  take part.
- Decided alone during PIC31.0 (2026-11-02), for the owner to review: (1) the gate failed, so the
  work stopped there as the plan says; nothing from PIC31.1 on was started. (2) The spike's scenes and
  tests stay in the repo (`spike_scene.rs`, `tests_soft_spike.rs`; the timing tests are
  `#[ignore]`d), so the next attempt reruns them unchanged. (3) The diagnostic patched copy of rapier
  was never committed. (4) This container's disk (about 30 GB writable) holds one workspace build,
  not three, so builds ran one at a time with `CARGO_INCREMENTAL=0`.

### Later

- Presets: rope/string, inflatable (pressure), clay (keeps dents, plasticity), tearable paper.
- Linkages (joints between objects, tlprocedural's `LinkageBindingUtil`/`LinkagePhysicsSync`).
- Physics on modifier copies (each copy its own body).
- Kinematic bodies moved by keyframes, once the clock grows a timeline.
- Canvas preview of colliders and the walls (tlprocedural's `PhysicsBoundsOverlay`).
