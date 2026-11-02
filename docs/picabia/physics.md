# Physics

**Status:** 2026-11-02, phase 1 (PIC30.1, PIC30.2): drop-and-settle; phase 2 (PIC31, soft bodies
and live simulation) planned, not started (see [Phase 2](#phase-2-soft-bodies-and-live-simulation-planned)). Ported from the owner's
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

Planned 2026-11-02 with the owner; not started. Any object (or several) can be given a soft-body
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

### Clones and Apply

- While a simulation session is open, the canvas draws each body's clone at the current moment; the
  originals are dimmed. Clones aren't selectable and aren't exported or saved.
- **Rigid clone:** the art moved and turned (phase 1's transform). **Cloth clone:** the art bent
  along the strip's simulated centre line (each point placed by its distance along the axis and its
  offset across it). **Jelly clone:** the art carried through the simulated cells (each point by its
  position in its rest cell), with curves subdivided so straight sides bend smoothly
  (`vectorcraft_doc::live::map_nonlinear_within`, as Deform uses).
- **Apply** bakes the shown pose into every body in the session and closes it (one undo step,
  "Apply Physics"). **Cancel** closes it and leaves the art untouched. A live rectangle or ellipse a
  soft body bends becomes a path; a rigid one stays live.

### The clock and the transport bar

- A slim transport bar (the "slim bar now, keyframe timeline later" of `plan/reference/decisions.md`):
  play/pause, reset, a scrubber with the time, Simulate to Rest, Apply and Cancel.
- 60 steps per second, as phase 1. A frame is drawn with the newest finished step, so a slow scene
  plays slower than real time but stays exact.
- **Scrub:** the session keeps a world snapshot every second in memory (capped; older ones thinned).
  Seeking restores the nearest earlier snapshot and steps forward.
- **Pokes and setting changes are recorded** at their step, so scrubbing back and forth replays the
  same motion. Playing or poking after scrubbing back drops what was recorded after that moment.
- **Poke:** while the bar is open, dragging on a clone with the Selection tool pulls it by a spring
  (a soft body by its nearest particle, a rigid body by the grabbed point); letting go throws it.
- **Settings while playing:** a change in the physics row is applied to the running world at the
  next step (material, gravity, bounce, friction, damping). A change that needs a new mesh
  (resolution, preset kind) restarts that body from its rest pose, with a note.

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

The phase 1 fields still apply (gravity scale, bounce, friction, extra mass, sensor). Pins are stored
as points in the object's own space plus an optional object id to attach to, not as anchor indices,
so they survive editing the path; at build time each pin takes the nearest particle. Pinning: Direct
Selection on anchors, then Object ▸ Physics ▸ Pin Points; Attach Points To… picks the object (it
must have physics settings; a rigid body carries the pinned points with it).

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
| PIC31.1 | Soft settings: `bodyType: "soft"`, presets and overrides, pins; lenient parse, round trip |
| PIC31.2 | Bodies from objects: cloth strip and jelly/rubber cells, caps, junk; tune the presets |
| PIC31.3 | Pose to art: rigid, cloth and jelly warps; Apply as one undo step |
| PIC31.4 | Session and clock: world, steps, snapshots, event log, `physics.session/play/pause/reset/seek/apply/cancel`, rest detection |
| PIC31.5 | Transport bar and clones on the canvas; Simulate to Rest; rigid bodies on the same flow |
| PIC31.6 | Poke (Selection tool drag while the session is open) and `physics.poke` |
| PIC31.7 | Settings while playing (applied next step; re-mesh restarts the body) |
| PIC31.8 | Pins and attachments: Pin Points, Attach Points To…, canvas markers |

Budgets (targets, to be measured in PIC31.0 and kept by `vectorcraft-cli perf`): live at 60 steps a
second natively with 20 soft bodies of up to 400 particles and 200 rigid bodies; the web build at
least half that.

### Open questions

- Is the session saved with the file (settings, pins, recorded pokes and the current time), or is it
  a working state that closes on save? Proposed: not saved; Apply or Cancel before saving, and a
  save with an open session asks.
- Does Apply bake every body in the session, or can it bake only selected ones and keep the rest
  running? Proposed: every body first; Apply Selected later.

### Later

- Presets: rope/string, inflatable (pressure), clay (keeps dents, plasticity), tearable paper.
- Linkages (joints between objects, tlprocedural's `LinkageBindingUtil`/`LinkagePhysicsSync`).
- Physics on modifier copies (each copy its own body).
- Kinematic bodies moved by keyframes, once the clock grows a timeline.
- Canvas preview of colliders and the walls (tlprocedural's `PhysicsBoundsOverlay`).
