# Physics

**Status:** 2026-11-01, phase 1 (PIC30.1, PIC30.2): drop-and-settle. Ported from the owner's
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
  the same document simulates to the same bits on every machine and in the web build (checked: a
  20-body scene gives the same FNV hash of every result coefficient natively and in
  wasm32-unknown-unknown under Node). No threads, no `fem`.
- **Steps.** 60 steps per simulated second (tlprocedural's update rate). The run stops when every
  dynamic body sleeps, or at the step cap (default 3,000 = 50 s; 1–20,000). One run may do at most 6
  million body-steps (1,000 bodies get at most 6,000 steps), and at most 1,000 bodies take part. No
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

Caps: 2,000 outline points per object (simplified with Ramer–Douglas–Peucker, as tlprocedural's
`simplifyVertices`), 512 pieces (more: the object is its convex hull). Geometry that isn't finite,
is empty, reaches beyond ±1,000,000 pt or is smaller than 0.01 pt both ways is skipped with the
reason. A body of open outlines only gets the mass and inertia of its bounding box.

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
- **Not ported:** spawners, throwing, linkages (joints), the physics playback controller.

## Phase 2

- Linkages (joints between objects, tlprocedural's `LinkageBindingUtil`/`LinkagePhysicsSync`).
- Live simulation with a clock: play, pause, scrub, with the canvas drawing bodies as they move,
  then commit or cancel; kinematic bodies moved by their owner.
- Physics on modifier copies (each copy its own body).
- Soft bodies (rapier 0.36's `SoftBodySet`: ropes, blobs) as `bodyType: "soft"`: the settings schema
  already keeps unknown body types.
- Canvas preview of colliders and the walls (tlprocedural's `PhysicsBoundsOverlay`).
