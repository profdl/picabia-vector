# Modifiers

> The modifier spec, ported from picabia-craft (`docs/picabia/modifiers.md` there, params as of its
> P8) and rewritten for VectorCraft. The stack crate (`crates/modifiers`, PIC1.1) follows this page;
> drawing, the panel rows, Apply and the canvas handles have been ported since (see [`ROADMAP.md`](ROADMAP.md)).

A **modifier stack** is a list of live, non-destructive steps attached to an object. The object keeps
its own geometry; the stack describes how it is repeated, transformed and (with the geometry rows,
PIC20) reshaped. Edit the object or any
modifier and every copy follows. Paths, compound shapes, text and groups can carry a stack. Copies
are drawn, exported and hit-tested as part of their source object: clicking a copy selects the
source.

## Where to find modifiers

The Appearance panel isn't open in the Essentials workspace, so modifiers are reachable from the
places that are (PIC3.4):

- **Properties panel ▸ Modifiers.** A section between Appearance and Align, shown whenever objects
  are selected. Its header shows the number of rows and collapses the section (remembered for the
  session). Open, it shows the same rows as the Appearance panel (eye, kind and copy count such as
  "Linear Array ×6", the disclosure that opens the row's fields, the `⋯` menu, the copy count and
  **Apply**; one "Modifiers: Mixed" row when several selected objects' stacks differ), then
  **+ Add Modifier**.
- **The Add Modifier menu** (one builder, `picabia::add_menu`), grouped: **Repeat** (Linear Array,
  Circular Array, Grid Array, Mirror, Wallpaper, Copy to Shape, Transform Each, Randomize),
  **Shape** (Resample, Trim, Tidy, Deform, Displace, Screen, and the screen presets Ben-Day Dots
  and Line Fill) and **Make** (Stencil, Physics ▸ Add Physics). Each entry runs `modifier.add` on
  the selection, one undo step. The same menu is Add New Effect (`fx`) ▸ Modifiers in the
  Appearance panel and the Properties panel's `fx` button, and Object ▸ Modifiers lists the same
  entries in the same groups (separators instead of headings).
- **The Contextual Task Bar's Modifiers button** (a stack-of-squares glyph, after the bar's labelled
  buttons and before the Fill chip) opens the Add Modifier menu; when a selected object has
  modifiers, the menu starts with **Show Modifiers** (opens the Properties panel with the section
  open and scrolled to), **Apply Modifiers** and **Remove All Modifiers**. The button is left out
  when the bar would no longer fit the canvas with it.
- **The canvas badge.** When one to eight selected objects have a stack, each shows a small tag
  just outside its bounding box's top-right corner (8 points out on the diagonal, clear of the
  handles): the glyph and the number of rows, tooltip "3 modifiers — click to show". It is drawn
  in screen points (it doesn't scale with the zoom), with the selection tools only, and only when
  the task bar may show (not on the home screen, in Presentation Mode, while a 3D revolve is
  edited, while the Simplify or Smooth slider shows, nor while something is dragged); it gives way
  to a floating panel or menu over it. A click does what Show Modifiers does: it opens the
  Properties panel (leaving it open wherever it already shows) with the section open and scrolled
  into view.
- **The command palette** finds each kind and preset by name ("Linear Array", "Add Circular
  Array", "Ben-Day Dots", "Screen", "Stencil", "Simulate Physics"): each kind and preset
  has a command of its own, `modifier.add.<kind>` / `modifier.add.benDay` / `modifier.add.lineFill`
  (Object › Modifiers in the palette and the shortcut editor, so a key can be given to one), which is
  `modifier.add` with that kind filled in (a different `kind` or `preset` is refused; the undo step
  is named for it, "Add Linear Array"). "Simulate Physics" and "Make Stencil" were already
  commands of their own.

## How Picabia differs on VectorCraft

picabia-craft built the stack on FigCraft. On VectorCraft the spec below holds, with these
differences:

- **Storage.** The stack is a VectorCraft **foreign attribute** on the object: local name
  `picabia.modifiers`, namespace `https://github.com/profdl/picabia-vector/ns/modifiers` (prefix
  `picabia`), holding the same JSON string as before:
  `{"v":1,"stack":[{"id":"m1","enabled":true,"kind":"linearArray", ...}]}`. VectorCraft keeps
  foreign attributes through save, copy, paste, duplicate and undo and never reads them, so plain
  VectorCraft keeps the stack and draws the object without its copies.
- **Instance transforms are in document space.** VectorCraft objects keep their geometry in document
  space and have no transform of their own (FigCraft layers had one, inside their parent's space).
  A copy is the object's geometry with the instance transform applied, document space to document
  space; the identity is the original. The pivot is the centre of the object's geometric bounds.
- **One list in the Appearance panel.** Picabia's rows are entries in VectorCraft's Appearance
  panel, in one ordered list with VectorCraft's own effects, built from VectorCraft's widgets (the
  Properties panel's Modifiers section shows the same rows; see Where to find modifiers). Evaluation rule: **Picabia rows run in their order, top first**, as
  below. A VectorCraft geometry effect **below** a copy row (an array, mirror, wallpaper or copy to
  shape) applies to **each copy**; one **above** it applies to the object **before** it is copied.
  Until the interleaving is built, the simplest correct ordering that keeps this rule is shipped:
  the Picabia rows are one block in the list, between the fills and strokes and the object's own
  effects, and every VectorCraft effect counts as below the copy rows (it draws on each copy; see
  How copies are drawn). A row's eye turns it off, its disclosure shows its fields, its `⋯` menu
  moves, duplicates, copies, pastes, removes it or applies the stack down to it; under the rows are
  the evaluation's warnings and Apply. Add New Effect (`fx`) ▸ Modifiers and Object ▸ Modifiers
  add each kind.
- **Parameters, not variable bindings.** picabia-craft's P6 bound modifier fields to FigCraft
  variables (`Node.bound`, `variable.bind`). Here P6 is the **parameters** design (PIC6): number and
  text parameters in VectorCraft's Variables panel (a data set can supply their values), and a chip
  on every number field binding it to a parameter or an expression (ramps later), kept in the row's
  `bind`. See [`parameters.md`](parameters.md). The per-field expressions below (`<field>Expr`) stay
  in the stack format.
- **No generator rows.** picabia-craft's `imageTrace` and `pathfinder` generator rows (its P13, P14)
  are not ported: VectorCraft's own Image Trace and live Pathfinder are used, and a stencil can sit
  on a traced image.

## Evaluation order

**The top row runs first**, like a recipe. Each step acts on everything the step above produced:

- **Array steps** (linear, circular, grid, mirror, wallpaper, copy to shape) repeat the **whole
  previous set as a block**. Grid, then Circular is a ring of grids. Copy 0 of an array is the
  identity: the original is one of the `count` copies (copy to shape doesn't keep the original).
- **Per-instance steps** (transform each, randomize, color) change each instance (around its own
  pivot; color changes its colours instead) and do not add copies. They always act on the **final** copies, after every array step has run,
  wherever their row sits in the stack: each copy gets its own values, and `i`/`n` count all the
  copies. (Their position among the arrays doesn't change the result; their order among
  themselves does. Color moves nothing, so its order matters only among Color rows: a second one
  shifts the colours the first made.)
- **Geometry steps** (resample, trim, tidy, deform, displace, screen; PIC20) change paths instead of placing copies. A
  geometry step **above every copy step** (array or per-instance) changes the object itself before
  it is copied: every copy is a copy of the changed object. A geometry step **below a copy step**
  runs on **each finished copy, after its transform**, in document space, in stack order (whether
  it sits between two arrays or after the last: like the per-instance steps, it acts on the final
  copies). A stack of geometry steps alone draws one reshaped copy. See *Geometry rows* below.
- A **stencil** step ends the stack (see `stencil` below): it adds no copies, and the enabled steps
  below it are ignored with one warning. Geometry steps above it reshape what it cuts.
- A disabled step is skipped. An empty or fully disabled stack draws the object normally.
- Limits: at most 64 steps, `count` at most 1000 per array, and at most **10 000 instances** per
  object in total. A stack that would exceed this is cut at the last step that fits, with a warning.

## Pivot and units

- Pivot: the centre of the object's geometric bounds, in document space. Mirror axes and
  per-instance rotation and scale use it. The circular array's `centerX`/`centerY` are an offset
  from this pivot.
- Lengths are points, angles are degrees (positive turns clockwise on screen, y pointing down as in
  VectorCraft's document space), scale is a factor (1 = 100%).
- Copies are placed in document space: `copy = instance * geometry`.

## Index, n and t

Every copy has a flat **index** `i` (0 = the original). After stacking, each array step repeats the
previous block, so `i = k * n_prev + i_prev` where `k` is the copy number within that step. `n` is
the total number of copies and `t = i / (n - 1)` (0 when `n = 1`) runs from 0 to 1 across them.
`index`, `n` and `t` drive the progressive ramp, the randomize seed and per-copy expressions (below).

## How copies are drawn (A4)

An object with an active stack draws as its **art**: a group that takes the object's place and
identity and holds one copy per modifier instance (`vectorcraft_render::picabia::art`). Each copy is
the object with its geometry transformed by the instance transform, as `modifier.apply` bakes it:
strokes, effects, live corners and pattern tiles scale with the copy. The same art is what the
canvas draws, what bounds, culling and dirty regions measure, what every vector export writes and
what clicks, hover and marquees hit (a copy hits as its object).

- **Effects on each copy.** The stack makes the copies from the object's geometry first; each copy
  then draws with the object's fills, strokes and effects (geometry effects and raster effects such
  as a drop shadow apply to each copy).
- **Transparency once.** The object's opacity, blending mode, isolation and opacity mask apply once,
  to all its copies together (as for Crop Marks): overlapping copies of a half-transparent object
  don't darken each other. (picabia-craft composited each copy on its own.)
- Members of a copied group keep their own stacks, within one budget (`MAX_NESTED_COPIES`, 10 000
  objects drawn, the counts multiplied across nesting levels; past it the members draw once in each
  copy). An object whose copies would clone more than 100 000 objects (`MAX_ART_NODES`, as
  `modifier.apply`'s cap) draws once, and `modifier.list` says why.
- Copies never knock each other out (members inside each copy still do, as the object says).
- `modifier.apply` bakes exactly what is drawn: the same pivot, the copies inside a group that takes
  the object's opacity, blending mode and opacity mask.
- An object whose stack makes no copies (no active row, a single unmoved copy without geometry
  rows, a missing copy-to-shape target) draws exactly as VectorCraft draws it without the stack.
- **Geometry rows** (PIC20) run inside the same builder (`vectorcraft_modifiers::Shaper`): the rows
  above every copy row on the object before it is cloned, the rows below on each copy after its
  transform. So the canvas, bounds, every export, `modifier.apply` (which bakes the reshaped
  paths), the stencil and the laser all see the same reshaped paths. A click on reshaped copies
  tests the art as drawn (a transform of the object isn't what is drawn); the hover outline follows
  the reshaped paths. What a row couldn't do (over the anchor cap, type passed over) shows under the
  rows.

## Export

Every vector export (SVG, PDF, print, EPS, DXF, EMF, WMF, legacy `.ai`, plug-in exporters) writes
the copies as plain objects, each baked as any object is, with an export warning; PNG and the other
raster exports draw them. **The written copies carry no stack**, so reading such a file back gives
the copies as plain objects, never copies of copies. The live stack survives where the file carries
VectorCraft's own document: the native `.vectorcraft` file (the foreign attribute), SVG and PDF
saved with Preserve Editing, and an EPS that VectorCraft wrote. (picabia-craft's SVG never carried
plugin data, so it had no such choice to make; writing the stack on the source next to its written
copies would double them when the SVG is read back.)

## Parameters

Stored as the foreign attribute above. Fields are flat camelCase next to `kind`. Unknown kinds and
fields survive a save and are ignored when drawing. The tables list defaults and the range a field
is clamped to; non-finite numbers are refused. "Layer" below means the object carrying the stack.

### linearArray

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `count` | copies | 1 to 1000 | 3 | Total copies, original included |
| `offsetX` | pt | any | 100 | Step between copies, along x |
| `offsetY` | pt | any | 0 | Step between copies, along y |
| `rotateEach` | deg | any | 0 | Extra rotation added per copy, about the pivot of each copy |
| `scaleEach` | factor | > 0 | 1 | Scale multiplied in per copy (1 = no change) |

### circularArray

The original is copy 0 and sits on the ring at `startAngle`; the other copies follow around it.

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `count` | copies | 1 to 1000 | 6 | Total copies, original included |
| `radius` | pt | >= 0 | 100 | Ring radius. The ring centre is this far from the original, opposite the start direction |
| `startAngle` | deg | any | 0 | Angle of the first copy (0 = right of the ring centre, positive turns clockwise on screen) |
| `sweep` | deg | -360 to 360 | 360 | Angle covered. At 360 or -360 copies are spaced `sweep / count` apart, so the last never lands on the first. Below that the last copy lands exactly at the end of the sweep |
| `centerX` | pt | any | 0 | Moves the whole ring (and with it the first copy), x |
| `centerY` | pt | any | 0 | Moves the whole ring, y |
| `align` | on/off | | on | Turn each copy by the ring angle so it keeps facing along the ring |

### gridArray

Copies run row by row from the original at the top left.

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `rows` | copies | 1 to 1000 | 3 | Rows (rows x cols x earlier copies is capped at 10 000) |
| `cols` | copies | 1 to 1000 | 3 | Columns |
| `gapX` | pt | any | 100 | Space between neighbouring copies' bounds, x (the step is the layer's width plus the gap) |
| `gapY` | pt | any | 100 | Space between rows, y (the step is the layer's height plus the gap) |

### mirror

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `axis` | choice | `vertical`, `horizontal`, `both` | `vertical` | `vertical` flips left to right across a vertical line; `horizontal` flips top to bottom; `both` gives four sets (original, left-right, top-bottom, both) |
| `offset` | pt | any | 0 | Moves the axis line away from the pivot, perpendicular to it (for `both`, each line moves this far) |
| `keepOriginal` | on/off | | on | On: previous set plus its reflection(s). Off: only the reflection(s); `both` then gives three sets |

### wallpaper

The 17 plane (wallpaper) symmetry groups. The layer is the motif in cell 0 at the identity; the
other copies are the group's motif transforms in cell 0 plus the same set in every other cell of a
`cols` x `rows` lattice (copies run cell by cell, columns fastest, so copy 0 is the original).
Rotation centres and mirror or glide axes pass through the layer's **top-left corner** (plus
`originX`/`originY`). A motif tiles without overlapping itself only when it fits the group's
fundamental domain (the cell divided by the copies per cell, next to the origin): a cell-sized
motif is fine for p1 but overlaps its own rotations and reflections in the others. Guides draw the
lattice lines through the origin, from the first to the last line the copies touch (reflections
that land before the origin are enclosed too). Copies per cell: p1 1, p2 2, pm 2, pg 2, cm 2, pmm 4, pmg 4, pgg 4, cmm 4, p4 4,
p4m 8, p4g 8, p3 3, p3m1 6, p31m 6, p6 6, p6m 12. The centred groups (cm, cmm) use their primitive
cell, a parallelogram with edges `(w, 0)` and `(w/2, h/2)`.

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `group` | choice | `p1` `p2` `pm` `pg` `cm` `pmm` `pmg` `pgg` `cmm` `p4` `p4m` `p4g` `p3` `p3m1` `p31m` `p6` `p6m` | `p4m` | The symmetry group |
| `cols` | cells | 1 to 100 | 3 | Cells along the first lattice vector |
| `rows` | cells | 1 to 100 | 3 | Cells along the second lattice vector (cols x rows x copies per cell x earlier copies is capped at 10 000) |
| `cellWidth` | pt | 0.001 to 1 000 000 | 100 | Lattice width, every group. For the square groups (p4*) it is the side; for the hexagonal groups (p3*, p6*) it is the lattice constant |
| `cellHeight` | pt | 0.001 to 1 000 000 | 100 | Lattice height: p1, p2, pm, pg, cm, pmm, pmg, pgg, cmm only. Hidden for the square and hexagonal groups, which it can't shape |
| `angle` | deg | 1 to 179 | 90 | Angle between the lattice vectors: p1 and p2 only (the other lattices have their angle fixed by the group); hidden for the rest |
| `originX` | pt | any | 0 | Moves the symmetry origin from the layer's top-left corner, x. Every group but p1 (which has no symmetry element to move; hidden there) |
| `originY` | pt | any | 0 | Same, y |

Which inputs apply: `group`, `cols`, `rows` and `cellWidth` to all 17; `cellHeight` to the oblique,
rectangular and centred groups; `angle` to p1 and p2; `originX`/`originY` to all but p1. The panel
hides the fields a group ignores and keeps their values, so switching groups and back loses
nothing; the evaluator ignores them too.

### copyToShape

Places copies of everything above it on another layer, the **target**: its outline (its own
shape; a group or compound shape is the outlines of its objects; not the target's own modifier copies),
in document space. Each copy puts the layer's pivot on a point of the outline, scaled by
`scale` around the pivot and, with `align`, turned to the outline's direction there. The original is
not kept: every copy sits on the target (the layer is still selectable where it is). Like the other
arrays it repeats the whole earlier set at each point, so a linear array above it puts that row of
copies at every point. Moving, resizing or editing the target moves the copies (the evaluation cache
keys on a hash of the target's outline, and the canvas repaints the copies when the target changes).
A hidden target works (handy for guide paths).

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `target` | layer id | text | none | The target layer's id (`"12"`; a number reads too). Empty: no copies, a warning asks for one |
| `mode` | choice | `points`, `alongPath`, `populate` | `alongPath` | `points`: one copy at each anchor (segment ends; a closed outline's start once). `alongPath`: `count` copies evenly spaced by arc length, over every subpath in order; a closed outline spaces them `length / count` apart (none doubles the first), an open one `length / (count - 1)` so both ends get one. `populate`: `count` copies at seeded pseudo-random points inside the outline (nonzero winding) |
| `count` | copies | 1 to 1000 | 10 | Copies for `alongPath` and `populate` (`points` makes one per anchor, at most 1000) |
| `scale` | factor | 0 to 100 | 1 | Scale of every copy, about its pivot |
| `align` | on/off | | on | `points` and `alongPath`: turn each copy so its x axis follows the outline (the outgoing segment at an anchor; an open outline's last anchor uses the incoming one). `populate` ignores it |
| `seed` | integer | 0 to 4 294 967 295 | 1 | Seed for `populate`; same seed, same points everywhere |

**Refused targets.** `modifier.add`/`modifier.set`/`modifier.paste` refuse a target that is the
layer itself, an ancestor or descendant of it, a missing object, or
one whose own stack copies back onto this layer (directly or through other copy-to-shape targets).
A file that has such a target anyway (or whose target was deleted later) draws the layer without
copies and the panel shows the reason; other edits to the layer still work, and undoing the delete
brings the copies back. `target` can't be bound to a variable.

**Copying layers.** Duplicate and copy and paste point a copied source at the copy of its target
when both were copied together; a source copied alone keeps copying onto the original target. (In
picabia-craft this also covered page duplicates and component instances; VectorCraft's symbols are
checked when copy to shape is wired up here.)

**Picking a target.** `modifier.targets {id, modifier?}` lists the layers a layer can copy onto (and
whether the row's current target is valid); the row's target dropdown in the Appearance panel offers the same list.

### transformEach

The change happens around each copy's own pivot and in its own frame (so a copy that was turned
moves along its turned axes).

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `rotate` | deg | any | 0 | Rotation of every instance |
| `scaleX` | factor | 0 to 100 | 1 | Horizontal scale of every instance |
| `scaleY` | factor | 0 to 100 | 1 | Vertical scale of every instance |
| `offsetX` | pt | any | 0 | Shift of every instance, x |
| `offsetY` | pt | any | 0 | Shift of every instance, y |
| `progressive` | on/off | | off | Scales the effect by `t`: the first copy is untouched and the last gets the full amount (offset and rotation grow linearly, scale goes from 1 to the full factor). One linear ramp; ease shapes are not implemented |
| `jitter` | factor | 0 to 10 | 0 | Each amount above is multiplied by `1 + jitter x r`, `r` between -1 and 1 drawn per copy and amount (0 = off) |
| `jitterSeed` | integer | 0 to 4 294 967 295 | 1 | Seed for `jitter` |
| `offsetXExpr`, `offsetYExpr`, `rotateExpr`, `scaleXExpr`, `scaleYExpr` | text | | none | Per-copy expressions (see Expressions) |

### randomize

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `position` | pt | >= 0 | 0 | Maximum random shift in x and y |
| `rotation` | deg | >= 0 | 0 | Maximum random rotation, plus or minus |
| `scale` | factor | >= 0 | 0 | Maximum random scale change, plus or minus (0.2 = 80% to 120%) |
| `seed` | integer | 0 to 4 294 967 295 | 1 | Same seed, same result on every platform and in the browser |
| `positionExpr`, `rotationExpr`, `scaleExpr` | text | | none | Per-copy expressions for each amount (see Expressions) |

Randomize derives its numbers from `seed` and the copy index only (SplitMix64 inside the crate, no
platform generator), so a copy never changes when other copies are added after it.

### color

Gives each copy its own colour (PIC32; tlprocedural's Color and Color Each in one row; the full
spec, maths and choices are in [`colour.md`](colour.md)). It shifts the OKLCH hue, chroma and
lightness of every fill and stroke of each copy (solid colours, gradient stops, mesh points, a
colorized image's colour, type), by the same amount on every copy, ramped across the copies, at
random per copy, or all three. It runs on the final copies like the other per-instance steps and
adds no copies, so bounds, clicks, the stencil's islands and the laser's paths don't change; on an
object without copy rows it recolours the object itself. Pattern tiles (with a note), images'
pixels, effects' colours, None and Registration keep their colours; a shifted colour loses its
swatch link; RGB, CMYK, Gray and Lab keep their model (a Gray that gains chroma, and a spot colour,
become process colours of the document's mode). A stencil below reads the shifted colours.

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `hue` | % | -100 to 100 | 10 | Fill hue shift; 100 = a full turn |
| `chroma` | % | -100 to 100 | 10 | Fill chroma shift; 100 = +0.4 OKLCH chroma |
| `lightness` | % | -100 to 100 | 10 | Fill lightness shift; 100 = +1 OKLCH lightness |
| `stroke` | enum | `same`, `own`, `none` | `same` | Strokes take the fill's shifts, their own, or keep their colour |
| `strokeHue`, `strokeChroma`, `strokeLightness` | % | -100 to 100 | 0 | Stroke shifts with `stroke: own` |
| `progressive` | on/off | | on | The ramp: each copy gets `shift × t × amount` (`t` = 0 on copy 0, 1 on the last; a lone object gets `amount`). Off: the full shift on every copy |
| `amount` | factor | 0 to 1 | 1 | Ramp intensity |
| `reverse` | on/off | | off | The ramp runs the other way |
| `order` | enum | `copies`, `array`, `leftRight`, `topBottom`, `diagonal` | `copies` | What the ramp and jitter count: all the copies, one array row's copy number, or the copies' centres left to right, top to bottom or by `x + y` (ties keep copy order) |
| `array` | integer | 1 to the copy rows | 1 | With `order: array`: which copy row (1 = the first from the top) the ramp runs along |
| `jitterHue`, `jitterChroma`, `jitterLightness` | % | 0 to 100 | 0 | Random extra fill shift per copy, plus or minus |
| `strokeJitterHue`, `strokeJitterChroma`, `strokeJitterLightness` | % | 0 to 100 | 0 | Stroke jitter with `stroke: own` |
| `seed` | integer | 0 to 4 294 967 295 | 42 | Jitter seed: per copy and channel (SplitMix64), stable as copies are added and the same in the browser |
| `hueExpr`, `chromaExpr`, `lightnessExpr` | text | | none | Per-copy expressions for the fill shifts, replacing `field × ramp` (see Expressions) |

Presets (`modifier.add` `preset`, and Add Modifier ▸ Color): **Rainbow** (hue 100: one full turn
across the copies), **Fade** (lightness 60: towards white), **Vary** (ramp off; jitter hue 8,
chroma 10, lightness 10).


### stencil

Turns the layer, with every copy the rows above it made, into a laser-cut **katagami** stencil: one
sheet of paper or film whose cut-outs let resist paste through. It is the last step: it adds no
copies (the copies above it stand), and enabled rows below it are ignored with one warning (only
the first enabled stencil counts). Apply bakes it into its sheet: the object becomes the paper as
drawn (one even-odd path with the cut-outs open; with the cloth preview, a group of the cloth and
the paste), copies above it included, rows below it dropped. Apply Up to Here on a row above it
bakes only those rows, and the stencil row moves onto the new group, live.

**Status here:** ported (picabia-craft P8 to P12, P17): the sheet, islands, ties, preview, laser
export and test card (`vectorcraft-stencil`; see [`ROADMAP.md`](ROADMAP.md)). The fields below are
picabia-craft's and stay the contract.

**On the canvas and in exports (PIC9.5b, PIC11.10).** Once a sheet is computed (in the background;
a stale one is shown until the new one lands), the canvas draws the object as its sheet: the paper
preview fills the sheet with the cut taken out (even-odd), the cloth preview fills the sheet rectangle
with cloth and the cut in paste colour. Bounds, clicks and marquees use the sheet rectangle (known
from the stack before any result exists), so the margin selects the stencil; before the first result
the design (or its copies, past a tile) draws and counts too, and a stale sheet counts where it is
drawn as well as where the new one will be. Direct Selection on a stencil drawn as its sheet selects
the stencil itself (the sheet has no members to pick). Results stay with their document: another
open document's stencil with the same object id draws its design until its own result lands. A
selected stencil shows its tie bars and, with a repeat, the dashed repeat cell. Every export through
`document.serialize` and the file exporters (not the native file or a template, which keep the
design and the stack) computes stale sheets first, then SVG, PDF and PNG draw the sheet; with tile
output the sheet is the repeat cell's tile, placed where it is cut.

**Islands and ties.** An island is a hole in the cut: the cut's contours nest (an outline, its
holes, cut-outs in those holes...), and every hole is the outline of a piece of paper. Its depth
counts the islands around it. In `ties` mode each island gets up to `tiesPerIsland` straight bars
`tieWidthMm` wide across the cut to paper already held: the outline of the cut-out it sits in, or
an island of the same cut-out tied before it (so a cluster of islands is tied outside in, each to
its neighbour). Bars go along the shortest gaps, run half a width into the paper on both sides, and
never touch another island, another cut-out or the sheet's edge. `tieSpread` keeps an island's
bars apart around it; `tieAngle` prefers gaps along that direction (or the opposite one), and
`tieSeed` decides between equally short gaps. After tying the islands are looked for again: any
still floating is counted in the stats (`untied`) with a warning. The bars (and the lattice that
became paper) are in the result for the preview overlay.

Lengths here are **millimetres** at the stencil's physical size, unlike the point-based steps:
`mm per point = widthMm / layer width` (the height follows the aspect ratio), and every `...Mm`
limit converts to layer units with that factor. Colours are `"#rrggbb"` texts (`#rgb` and any case
read too; they are written lowercase).

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `colourMode` | choice | `threshold`, `pick` (`separate` reads and saves, P12) | `threshold` | `threshold`: colours whose luminance (Rec. 709 on sRGB-decoded values, 0 to 1) is above `threshold` are cut, so lighter is cut. `pick`: the colours in `cutColours` are cut, every other colour stays paper. `separate` (one stencil per colour) evaluates as `threshold` with a warning for now. An unknown value reads as `threshold` |
| `cutColours` | list of colours | at most 256 | none | `pick`: the colours cut out (exact match after rounding to 8 bits). Entries that aren't colours are left out, repeats kept once; one colour text reads as a list of one |
| `threshold` | 0 to 1 | 0 to 1 | 0.5 | `threshold`: the luminance level |
| `invert` | on/off | | off | Swap what is cut and what stays paper (both modes) |
| `widthMm` | mm | 1 to 10 000 | 300 | The design's physical width |
| `marginMm` | mm | 0 to 1000 | 15 | Paper border around the design (the sheet is the design's bounds plus this) |
| `minCutAreaMm2` | mm² | 0 to 10 000 | 0.5 | Cut pieces and paper holes smaller than this are dropped |
| `minCutWidthMm` | mm | 0 to 100 | 0.3 | Cut slivers narrower than this are removed (an opening: shrink, then grow) |
| `minBridgeMm` | mm | 0 to 100 | 0.5 | Paper bridges narrower than this are removed (a closing: grow, then shrink) |
| `toleranceMm` | mm | 0 to 10 | 0.05 | Outline simplification |
| `preview` | choice | `paper`, `cloth` | `paper` | Canvas preview: the paper sheet with its cut-outs open, or the dyed cloth |
| `paperColour` | colour | | `#c8a165` | The sheet in the paper preview (kakishibu brown) |
| `pasteColour` | colour | | `#1f3a8a` | Where the paste goes through, in the cloth preview (indigo) |
| `clothColour` | colour | | `#f4efe6` | The undyed cloth, in the cloth preview |
| `tieMode` | choice | `ties`, `gauze` | `ties` | What holds the **islands** (paper pieces the cut separates from the sheet, such as a crescent inside a cut swoosh). `ties`: each island gets tie bars to the paper around it. `gauze`: no ties; the islands are kept (a gauze or silk mesh laid on the stencil holds them) and flagged. `pattern`: a lattice of thin paper bars across the cut (the `pattern...` fields) holds every island it crosses; islands it misses get ties as in `ties` mode. An unknown value reads as `ties` |
| `tieWidthMm` | mm | 0.05 to 50 | 1 | Width of each tie bar |
| `tiesPerIsland` | count | 1 to 8 | 2 | Tie bars per island, along the shortest cut gaps to the paper enclosing it |
| `tieSpread` | on/off | | on | Keep an island's ties apart (spread around it) instead of all on its very shortest gaps |
| `tieAngle` | degrees or none | wrapped to 0 to 360 | none | Preferred tie direction (0 = to the right, 90 = up on the canvas; the opposite direction counts too). `null` = no preference |
| `tieSeed` | whole number | | 1 | Breaks ties between equally short gaps (same result on every platform) |
| `patternType` | choice | `grid`, `horizontal`, `vertical` | `grid` | `pattern`: the lattice's bars. An unknown value reads as `grid` |
| `patternWidthMm` | mm | 0.05 to 50 | 0.6 | `pattern`: bar width (the tie size). Bars thinner than `minBridgeMm` give a warning |
| `patternSpacingMm` | mm | 0.5 to 500, at least twice the width | 8 | `pattern`: centre-to-centre spacing of the bars |
| `patternAngle` | degrees | wrapped to 0 to 360 | 0 | `pattern`: rotates the lattice (counter-clockwise on the canvas). The lattice is anchored at the sheet's top-left corner, so it stays put when the design changes |
| `patternWhere` | choice | `everywhere`, `islands` | `everywhere` | `pattern`: across every cut-out, or only in the cut-outs that border an island. An unknown value reads as `everywhere` |
| `material` | choice | `paper`, `film`, `card`, `custom` | `paper` | The sheet material. Setting a preset fills `minCutWidthMm`, `minBridgeMm`, `tieWidthMm` and `minCornerAngle` (unless the same edit sets them): paper 0.3 / 0.5 / 1 / 30 (the defaults), film (mylar, PET) 0.2 / 0.4 / 0.6 / 20, card 0.5 / 1 / 1.5 / 35. Changing one of those four away from the preset makes it `custom`. An unknown value reads as `paper` |
| `limitMode` | choice | `fix`, `flag` | `fix` | `fix`: clean-up removes narrow slivers and bridges and drops small pieces. `flag`: clean-up only simplifies; every cut or bridge that breaks the limits stays and is reported. An unknown value reads as `fix` |
| `minCornerAngle` | degrees | 0 to 179 | 30 | Paper corners sharper than this are flagged |
| `hoshi` | on/off | | on | Registration holes (hoshi) at the corners of the sheet, or of the tile |
| `hoshiDiameterMm` | mm | 0.2 to 20 | 1.5 | Diameter of the registration holes |
| `repeat` | choice | `auto`, `off` | `auto` | `auto`: when the last array row above the stencil is a `wallpaper` or `gridArray` (and no `transformEach` or `randomize` row varies the copies), islands and ties follow its repeat (a grid of one column or row repeats one way only). `off`: the design is one picture. An unknown value reads as `auto` |
| `output` | choice | `sheet`, `tile` | `sheet` | `sheet`: the whole design. `tile`: one repeat cell plus margin and registration holes, for step-and-repeat printing; without a repeat it cuts the sheet, with a warning. An unknown value reads as `sheet` |

Numbers outside their range are clamped when evaluating (a warning names the field); a colour that
doesn't read takes its default. `cutColours` can't be bound to a variable.

### Geometry rows: resample, trim, tidy, deform, displace, screen (PIC20)

Ported from tlprocedural (the owner's TypeScript app): its Subdivide and Rebuild Points become
**Resample**, Trim Path becomes **Trim**, Close Path and Sort Points become **Tidy** (the owner's
row list: Resample, Trim, Tidy). Ranges and defaults are tlprocedural's; where the port behaves
differently on purpose it says so below. They run as the evaluation order says: above every copy
row on the object, below one on each copy. Each subpath is treated on its own (tlprocedural used
only a shape's outer ring).

#### resample

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `mode` | choice | `subdivide`, `count`, `spacing` | `count` | `subdivide`: split every segment in two, `iterations` times. `count`: `count` points spaced evenly along each subpath by arc length (a closed one at least 3, the gap back to the start as long as the others; an open one keeps both ends). `spacing`: a point every `spacing` along each subpath from its start (a closed one: at least 3, the last gap shorter; an open one ends at its end). `count` and `spacing` join their points with straight segments, as tlprocedural's Rebuild Points with smoothing 0 |
| `method` | choice | `linear`, `catmullRom`, `chaikin` | `linear` | `subdivide` only. `linear`: a new anchor in the middle of each segment; **curves keep their exact shape** (tlprocedural flattened them to their anchors). `catmullRom`: each anchor, then the point of the Catmull-Rom spline through the anchors halfway to the next (tlprocedural's replaced the anchors with those points); straight segments. `chaikin`: corner cutting at each segment's quarter points (the path rounds off and shrinks a little); straight segments. A closed subpath of n anchors gets 2n, an open one 2n − 1 |
| `iterations` | times | 1 to 5 | 1 | `subdivide` only |
| `count` | points | 2 to 1024 | 20 | `count` only. (tlprocedural's Rebuild Points defaulted to `simplify`, not ported; `count` is the default here) |
| `spacing` | points | 1 to 5000 | 24 | `spacing` only. Fewer than three steps round a closed subpath spread its three points evenly (tlprocedural wrapped them round) |

`count` and `spacing` measure the true curve, not a flattened outline.

#### trim

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `start` | fraction of the length | 0 to 1 | 0.1 | Where the kept part starts (the panel shows a percentage) |
| `end` | fraction | 0 to 1 | 0.9 | Where it ends |
| `offset` | fraction | 0 to 1 | 0 | Added to both. On a closed subpath each is then wrapped into 0..1 (so the offset slides the window round it); on an open one each is clamped to 0..1 |

A closed subpath's window may run through its start point (end before start keeps the part through
it). An open subpath never wraps: start and end, offset added, are **clamped** to 0..1 (end 1 is its
end), then put in order (end before start keeps the part between them); an offset that slides the
window off the end leaves nothing. This differs from tlprocedural on purpose: tlprocedural wrapped
an open path's fractions too, so `end: 1` read as 0 and kept the part *before* `start`, and an
offset folded the window back to the start. The result is an open path; start = end (before the
offset) keeps the whole subpath (closed if it was), and a window shorter than the precision drops
the subpath. The cut is made on the true curve, so curves stay curves (tlprocedural
trimmed a flattened outline). The object keeps its fill: an open path is filled as if closed by a
straight line, as anywhere in VectorCraft (tlprocedural's trims drew no fill).

#### tidy

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `close` | choice | `off`, `straight`, `smooth` | `straight` | Close open subpaths. Ends within `tolerance` become one anchor (tlprocedural kept both, snapped together); otherwise `straight` adds a straight closing segment and `smooth` a curve that carries on both ends' directions, a third of the gap long at each end (tlprocedural added a nudged midpoint vertex). A subpath of two anchors whose ends meet is left alone |
| `tolerance` | points | 0 to 500 | 1 | Ends this close are joined |
| `sort` | choice | `none`, `angle`, `distance`, `x`, `y`, `random` | `none` | Put each subpath's anchors in order (after closing): by angle round the centre of its anchors, by distance from that centre, left to right, top to bottom, or shuffled by `seed` (a seeded shuffle; tlprocedural sorted with a random comparator, which differs between browsers). Equal keys keep their order. Sorted anchors are joined by straight segments; a closed subpath stays closed |
| `reverse` | on/off | | off | Reverse the order; with `sort: none`, the subpath's direction (curves kept) |
| `seed` | | 0 to 999999 | 1 | `sort: random` only |

#### deform

Ported from tlprocedural's Deform category (its Taper, Twist, Bend, Shear and Bulge/Pinch
modifiers, `src/store/modifiers/core/*ModifierProcessor.ts`, ranges and defaults from
`registry/distortionDefinitions.ts` and `modifierConfig.ts`) as **one row with a `mode`** (the
owner's one-block rows). Wave and Noise, in the same tlprocedural category, belong to the Displace
row (PIC20.4); Attractor Field isn't ported.

| Field | Unit | Range | Default | Used by | Meaning |
|---|---|---|---|---|---|
| `mode` | choice | `taper`, `twist`, `bend`, `shear`, `bulge` | `taper` | | Which deformation |
| `axis` | choice | `horizontal`, `vertical` | `horizontal` | taper, twist, bend | The axis it runs along |
| `falloff` | choice | `linear`, `easeIn`, `easeOut`, `smooth` | `linear` | taper, twist, bulge | How it eases: `t`, `t²`, `1 − (1 − t)²`, smoothstep. Taper and Twist ease along the axis (Twist from the middle out to each end); Bulge fades from its centre (1) to its radius (0) through the curve |
| `startScale` | factor | 0.1 to 3 | 1 | taper | The scale across the axis at its start (the panel shows %) |
| `endScale` | factor | 0.1 to 3 | 0.2 | taper | At its end |
| `angle` | degrees | −720 to 720 | 90 | twist | Between the two ends: the start of the axis turns −angle/2 about the box's centre, its end +angle/2, the middle not at all |
| `bendAngle` | degrees | −360 to 360 | 45 | bend | Every point turns about the bend point by `bendAngle` × (its fraction along the axis − `bendPoint`) |
| `bendPoint` | fraction | 0 to 1 | 0.5 | bend | Where along the axis it pivots (on the box's middle line) |
| `shearX` | factor | −2 to 2 | 0.3 | shear | x moves by `shearX` × the distance below the origin |
| `shearY` | factor | −2 to 2 | 0 | shear | y moves by `shearY` × the distance right of the origin |
| `origin` | choice | `center`, `topLeft`, `bottomLeft`, `topRight`, `bottomRight` | `center` | shear | The point that stays |
| `strength` | factor | −3 to 3 | 0.35 | bulge | Each point within the radius moves out from the centre by strength × extent × ½ × falloff (negative pinches in); the extent is how far the shape reaches from the centre |
| `radius` | factor | 0.01 to 2 | 1 | bulge | How far it reaches, a fraction of the extent (1 reaches the farthest point; the size of the push doesn't depend on it) |
| `progressive` | on/off | | on | all | Ramp across the array copies made above the row: the first gets the least of the effect (none, except Taper's: see below), the last `progressiveAmount` of it, evenly between |
| `progressiveAmount` | fraction | 0 to 1 | 1 | all, ramp on | |
| `progressiveReverse` | on/off | | off | all, ramp on | The first copy gets the most |

The ramp scales the angles, the shears and the strength, so the first copy gets none of them;
Taper's end scale moves towards its start scale, so the first copy is scaled across by
`startScale` throughout (unchanged only when `startScale` is 1).

**What the ramp counts** (the lead's decision, following tlprocedural): only **array** copies,
those of linear, circular and grid array rows. Stacked arrays read as one combined array, the
first array's copies counting fastest (as a copy's index does); the ramp runs from its first copy
to its last. Copies a mirror, wallpaper or copy to shape row makes (and the per-copy rows,
transform each and randomize, which make none) get the same ramp value as the copy they were made
from: a lone mirror gives both halves the whole effect; an array then a mirror ramps each half
0 to 1 alike; a mirror then an array ramps each mirrored pair together. Above every copy row (on
the object), and with no array or a single array copy, the whole effect is applied. Whoever builds
the copies passes each copy's place among all the copies (`Shaper::copy_at`), so skipping one
whose transform isn't finite doesn't shift the ones after it.

**Each copy in its own box and frame.** Below a copy row each copy is deformed within its own
bounds, read in its own frame: the copy's rotation (from its instance transform), and its
reflection when it has one, is taken out first and put back after, so the copies of a circular
array each taper along their own axis, as tlprocedural's do (it deformed each copy before rotating
it), and a mirrored copy is the mirror image of the deformed object. A group is deformed as one
box.

**Curves stay curves.** Shear is affine: anchors and handles move exactly and no anchors are added.
The others split each segment into pieces at most the box's diagonal ÷ 32 long (64 per segment at
most) and map their control points, VectorCraft's own distort-effect method
(`vectorcraft_doc::live::map_nonlinear_within`), so a rectangle's straight sides bend smoothly. The
pieces fit the anchor cap: where they would pass the copy's share, they grow longer (a coarser
curve) instead of the row being skipped. A deform that moves nothing (the first copy of a ramp,
except a Taper whose `startScale` isn't 1) keeps its anchors. A box smaller than 0.001 pt both ways (Taper, Twist), an axis shorter
than that (Taper, Twist, Bend) or a shape that reaches less than that from its centre (Bulge) is
left alone.

Differences from tlprocedural, on purpose:
- tlprocedural flattened every shape to a polygon (Bézier paths sampled every 6 px, a rectangle
  kept as its 4 corners, so a taper or twist of a rectangle only moved its corners) and wrote
  straight segments; here curves are mapped and straight sides are split, so they bend.
- Its box was the polygon's bounds; here it is the curve's true bounds.
- Bulge's centre is the box's centre; tlprocedural averaged the polygon's vertices, which moves
  with how densely each side happens to be sampled. Its extent is measured on the curve (anchors
  and 15 points along each segment) rather than the polygon's vertices.
- One `falloff` serves the three modes that ease (tlprocedural had three lists: Taper's linear,
  ease-in, ease-out, "smooth"; Twist's linear, ease-in, ease-out, ease-in-out; Bulge's linear,
  smooth, sharp). Taper's "smooth" and Twist's "ease-in-out" are the same smoothstep (`smooth`
  here); Bulge's "sharp" is `easeIn`; Bulge gains `easeOut`. Bulge defaulted to "smooth" in
  tlprocedural; the shared default here is `linear`.
- Bulge's `radius` is a factor (1 = 100%) like Picabia's other percentages; tlprocedural stored the
  percentage (1 to 200). Its smallest radius, 1%, is kept.
- The ramp counts array copies as tlprocedural's does (its `linearArrayIndex` / `arrayIndex`);
  stacked arrays are read as one combined array (tlprocedural read the most recent array's own
  index), and copy 0 is the original, as in tlprocedural.
- Bend keeps tlprocedural's maths (each point turned about the bend point by an angle that grows
  with its distance along the axis), which bends a long shape into a curve that tightens towards
  its ends rather than a circular arc; at the default 45° the two look alike.

#### displace

One row with five patterns (the handoff's D4), sharing a direction, an amplitude, a size and a
phase or seed. Ported from tlprocedural: its **Noise** modifier (PIC20.4; `NoiseModifierProcessor`,
`NOISE_DEFAULTS`, `NOISE_SCHEMA`: smooth noise and radial), its **Wave** modifier
(`WaveModifierProcessor`, `WAVE_DEFAULTS`, `WAVE_SCHEMA`), **Randomize Points**
(`applyRandomizeToPolygon`, `RANDOMIZE_DEFAULTS`: jitter) and **Subdivision Growth**
(`SubdivisionGrowthProcessor`, `SUBDIVISION_GROWTH_DEFAULTS`, `SUBDIVISION_GROWTH_SCHEMA`: fractal)
(PIC20.4c). Every anchor moves with its handles; the fractal also adds anchors. Roughen and Zig Zag
stay VectorCraft's own effects.

The pattern is saved as `mode` (its first two values' field), and every field added with PIC20.4c
defaults to what the row did before (`direction: own`), so a stack saved before reads and draws the
same bits (a test pins them).

| Field | Unit | Range | Default | Used by | Meaning |
|---|---|---|---|---|---|
| `mode` | choice | `spatial`, `radial`, `wave`, `jitter`, `fractal` | `spatial` | | The pattern (the panel's **Pattern**: Smooth noise, Radial ripple, Wave, Jitter, Fractal), below |
| `direction` | choice | `own`, `normal`, `x`, `y`, `angle` | `own` | all | Which way anchors move. `own`: the pattern's (tlprocedural's) own: smooth noise in both axes, radial and jitter away from the centre, wave in y across the width, fractal between the radial direction and the segment's normal. `normal`: along the path's outward normal at the anchor (square to the handles' line, or to the neighbours' chord at a corner; outward by the winding of the curve itself, handles included, so a two-anchor lens knows its outside). `x`, `y`, `angle`: along that axis of the copy (read in the copy's frame, so a turned copy moves turned) |
| `angle` | degrees | -360 to 360 | 0 | direction `angle` | From the copy's x axis, clockwise on screen |
| `amplitude` | points | 0 to 100 (fractal 0 to 200) | 10 | all | The most an anchor moves. Radial: percent of its distance from the centre (times a quarter). Fractal: the first step's push (tlprocedural's `displacement`, which reached 200). 0 leaves the path alone |
| `scale` | per point | 0.01 to 1 | 0.05 | smooth noise | The noise frequency (smaller: broader waves) |
| `octaves` | | 1 to 6 | 3 | smooth noise | Layers of noise, each twice as fine and half as strong, summed and normalised |
| `rippleCount` | | 2 to 12 | 4 | radial | Bumps round the centre |
| `waveform` | choice | `sine`, `square`, `triangle`, `sawtooth` | `sine` | wave | tlprocedural's `waveType` |
| `frequency` | cycles | 0.1 to 10 | 2 | wave | Cycles across the object (the panel's **Cycles**) |
| `phase` | degrees | 0 to 360 | 0 | wave | Shifts the wave along |
| `jitterMin` | share of the amplitude | -1 to 1 | 0 | jitter | The least an anchor moves (the panel's **Least move**, a percentage): each moves between `jitterMin × amplitude` and `amplitude`; -1 moves either way |
| `steps` | | 1 to 8 | 4 | fractal | Subdivision steps; each doubles the segments |
| `decay` | | 0 to 0.5 | 0.45 | fractal | Each step's push is this share of the one before |
| `wander` | | 0 to 1 | 0.15 | fractal | Turns each push by a seeded angle, up to an eighth of a turn either way (tlprocedural's `jitter`, renamed so it doesn't read as the Jitter pattern) |
| `edgeVariance` | | 0 to 1 | 0 | fractal | How far from a segment's middle the new anchor may sit (seeded; 0.1 to 0.9 of the way along) |
| `normalBias` | | 0 to 1 | 0.5 | fractal, not direction `normal` | 0 pushes away from the centre, 1 square to the segment (an axis direction takes this blend's share along it) |
| `inward` | on/off | | off | fractal | Push in instead of out |
| `seed` | | 0 to 999999 | 42 | not wave | Smooth noise: shuffles the noise's permutation table. Radial: the ripple's phase. Jitter: the random stream. Fractal: the split places and wander. Each drawn with the copy's place among the array copies (see *What the ramp counts* below) |
| `progressive` | on/off | | on | all | Ramp the amplitude across the array copies made above the row: none on the first, `progressiveAmount` of it on the last, evenly between |
| `progressiveAmount` | fraction | 0 to 1 | 1 | ramp on | The last copy's share (the panel shows a percentage) |
| `progressiveReverse` | on/off | | off | ramp on | The first copy gets the most |

**The patterns.**

- **Smooth noise** (`spatial`): each anchor moves by fractal simplex noise sampled where it sits:
  x by `fbm(x·scale, y·scale)`, y by `fbm(x·scale + 100, y·scale + 100)`, each times `amplitude`.
  Along a direction: the first of the two, times `amplitude`, along it.
- **Radial ripple** (`radial`): each anchor moves along the line from the centre of the anchors
  (their mean) by `distance × amplitude/100 × 0.25 × sin(rippleCount·angle + phase)`:
  `rippleCount` bumps round the centre. Along a direction: that amount, along it.
- **Wave** (`wave`): tlprocedural's `wavePolygon`. Each anchor's place `t` (0..1) across the
  object's extent (all its anchors) moves it by `amplitude × wave(t × frequency + phase/360)`.
  The wave runs across the direction it moves in: own and `y` across the width, moving in y
  (tlprocedural's horizontal axis, its default); `x` along the height from the bottom up, moving in x;
  `angle` across that angle, a quarter turn back from the way it moves (0° is `x`, 90° is `y`: one turning family). With `normal` the wave runs along each subpath's outline
  (`t` its share of the anchors' chord length from the first anchor) and moves out along the
  normal; round a **closed** outline the cycles are rounded to the nearest whole number (at least 1), so the wave meets itself where the outline starts (no seam; a square or sawtooth wave jumps there only as it does at every cycle). An extent under 0.001 pt (a column, for a wave across the width) is left alone, as
  there. Square, triangle and sawtooth are `computeWave`'s.
- **Jitter** (`jitter`): tlprocedural's Randomize Points. Each anchor moves by
  `amplitude × (jitterMin + u × (1 − jitterMin))`, `u` the next of tlprocedural's seeded LCG
  (one step per anchor, across the object's subpaths in order; none drawn when `jitterMin` is 1).
  Own direction: along the anchor's line from the centre, never through it (it stops 0.0001 pt
  short); an anchor at the centre stays (and draws nothing), as there.
- **Fractal** (`fractal`): tlprocedural's Subdivision Growth. `steps` times: every segment is
  split (in the middle, or at a seeded place with `edgeVariance`) and the new anchor pushed by
  `amplitude × decay^step` (in with `inward`): own direction, tlprocedural's blend of the
  direction from the subpath's area-weighted centre and the segment's normal (turned into the
  radial half-plane) by `normalBias`; `normal`, out along the segment's normal; an axis, the own
  direction's share along it. Then turned by the seeded `wander`. Segments shorter than 0.15 of the
  average are split but not pushed, as there. A closed subpath of n anchors ends with n·2^steps, an
  open one (n − 1)·2^steps + 1. The geometry cap (200,000 anchors for an object and all its copies)
  is checked **before** growing: a row that would pass it is skipped for that copy with the note
  "Displace would make more than 200000 anchors for this object and its copies; it is skipped"
  (lower `steps` or put the row above the array).

**What the ramp counts** is Deform's (above): only **array** copies (linear, circular, grid),
stacked arrays read as one combined array, the first array's copies counting fastest. Copies a
mirror, wallpaper or copy to shape row makes (and the per-copy rows, which make none) get the ramp
value of the copy they were made from: a lone mirror gives both halves the whole amplitude; an
array then a mirror ramps each half alike. On the object, with no array or with a single array
copy, the whole amplitude applies.

**Copies differ.** Below a copy row each copy is displaced in document space after its transform.
In `spatial` mode the noise is sampled where the copy sits, so copies in different places get
different noise (a mirrored copy gets the noise of where it lies, not its partner's mirrored). In
`radial` mode the phase is seeded by the copy's place among the array copies, the same place the
ramp reads, and the ripple's angle is read in the copy's own frame (its turn, and its reflection
when it has one, as Deform's): the two halves of a mirror pair share a phase and an amount and are
mirror images, and a rotated copy's ripple turns with it; copies with no array place use place 0. Above every copy row the object is displaced
once and every copy is that same shape. tlprocedural sampled page space and seeded radial by the
clone's array index, as here. Wave, jitter and fractal are read in each copy's frame too (its
axes, its reflection: a reflected copy's wander turns the other way), and seeded by its array place,
so a mirror pair is a mirror image in those patterns (tested).

**Deterministic.** The noise is a seeded simplex (after Stefan Gustavson's public-domain
description, as tlprocedural's own self-contained one; no noise library), its permutation table
shuffled by an integer LCG; the ripple, the wave, the jitter and the fractal use only `+ − × ÷ √`
(the angle's multiple by complex powers, sines by series, `decay^step` by products, tlprocedural's
mulberry32 and LCG in exact 32- and 64-bit integers), so the same stack gives the same bits on every
platform and in wasm.

Differences from tlprocedural, on purpose:

- **Curves stay curves.** tlprocedural flattened the shape to a polygon first (an ellipse became
  32 straight sides, a Bézier its sampled points) and displaced those vertices. Here each anchor
  moves with its handles, so a smooth path stays smooth. Straight-sided paths (rectangles,
  polygons) move exactly as tlprocedural's did; for a rougher outline put a Resample row above
  (by count or spacing), as tlprocedural's Subdivide before Noise.
- **The permutation table** is shuffled with exact integer arithmetic. tlprocedural's
  `buildPermTable` multiplied in doubles past 2^53 and lost bits, so one seed gives a different
  (equally stable) pattern here than there.
- **The radial phase** is drawn from the crate's seeded stream for `(seed, array place)`.
  tlprocedural took one LCG step of `seed + index`; its step is a fixed 0.514 of a turn, so
  copies alternated between two nearly equal phases (copies 0, 2, 4… looked alike) instead of
  each getting its own.
- **The radial centre** is the mean of the anchors of every plain path in the object (or copy).
  tlprocedural averaged the vertices of the flattened polygon (32 points round an ellipse, a
  Bézier's sampled points, a ring's repeated closing vertex counted once more), so on a path with
  unevenly spaced anchors the centre, and the ripple round it, can sit a little differently.
- **Not ported:** `animated` and `animationSpeed` (tlprocedural's panel showed them, its processor
  never read them; Picabia has no time line).
- **One row, shared fields.** tlprocedural's Wave, Randomize Points and Subdivision Growth were
  separate modifiers; here they are patterns of this row (the handoff's D4). Wave's `axis` became
  the shared `direction` (`y` horizontal, `x` vertical) and gained `angle` and `normal`; Randomize's
  `minDistance`/`maxDistance` in points became `jitterMin` (a share of the amplitude) and the
  amplitude; Subdivision Growth's `displacement` is the amplitude and its `jitter` is `wander`.
  Wave and jitter take the shared ramp (tlprocedural's Randomize had none; its Subdivision Growth
  defaulted the ramp off, the row's default is on).
- **Jitter**: tlprocedural's Randomize unioned the shapes first (a boolean modifier) and jittered
  the union's polygon round its centroid; here each path is jittered as it is, round the mean of
  the object's anchors, and its curves stay curves. The same stream on every copy there; here
  copies past the first draw their own (the crate's seeded stream for `(seed, place)`).
  tlprocedural picked a random seed when a row was added; the default here is 42, so a new row is
  reproducible.
- **Wave**: the square wave reads the cycle's half (tlprocedural took the sign of a sine, which
  differs only exactly on a half cycle); each copy waves across its own extent, in its own frame,
  as tlprocedural waved each instance unrotated. `x` runs up the height (tlprocedural's vertical axis ran down it: the same wave mirrored top to bottom) so that `x`, `y` and `angle` are one turning family; a wave round a closed outline takes whole cycles (tlprocedural had no outline wave). (PIC20.4d)
- **Fractal**: segments split as curves (de Casteljau), so a circle grows as curves; a straight
  segment's new anchor is a corner. Each subpath grows round its own centre (tlprocedural used the
  whole shape's), an open path grows its segments only (tlprocedural grew the outline of its
  stroke), and a closed subpath with no area is left alone, as there (no area counting its curves: the outside is read from the curve's own winding, handles included, so curves round anchors in a line still grow; a segment too short to have a direction pushes along the radial direction, not tlprocedural's fixed (0, 1)). The anchors are never
  thinned (tlprocedural thinned the source to 4096 / 2^steps points and capped each step at 4096):
  a row past the geometry cap is refused with a note instead. **Not ported:** repulsion (an
  iterative point-repulsion pass), intersection clean-up (a polygon-clipping self-union),
  growth bias and shape influence (they read other shapes on the page): so a high `wander` or
  `edgeVariance` can make the outline cross itself where tlprocedural's repulsion and union would
  have smoothed it. Segment seeds count anchors through the object's subpaths (tlprocedural's ring
  offset), doubling each step.
- Stacked arrays ramp as one combined array (tlprocedural read the most recent array's own
  index), as Deform's ramp does.

#### screen

Picabia's own vector halftone (PIC20.5c). **Not ported**: tlprocedural has no Screen; the row, its
contouring and its clipping were written for Picabia (the contouring is not derived from the AGPL
`marching-squares` package, nor from any other GPL/AGPL code). Each path becomes the dots of a
halftone screen laid over it, clipped to its outline.

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `spot` | choice | `round`, `ellipse`, `line`, `square`, `diamond`, `cross` | `round` | The spot in each cell (see the core below): round dots meet at mid-grey (a checkerboard) and turn into round holes in the shadows; ellipses chain along the angle first; lines thicken; squares and diamonds meet corner to corner then become holes; crosses thicken their arms |
| `lpi` | lines per inch | 1 to 300 | 20 | Cells `72 / lpi` pt apart |
| `angle` | degrees | -360 to 360 | 45 | The screen's turn, clockwise on screen |
| `quality` | samples a cell | 4 to 64 | 12 | How finely each cell is traced: higher follows thin lines and small dots more closely, and costs more |
| `tone` | choice | `fill`, `fixed`, `image` | `fill` | Where the ink amount (0 none … 1 solid) comes from, below |
| `amount` | % | 0 to 100 | 50 | `fixed` only: the tone everywhere |
| `image` | object id | | empty | `image` only: the image object whose darkness gives the tone |
| `invert` | on/off | | off | Ink the light parts: the tone becomes 1 − tone |

**Presets.** `modifier.add {"kind": "screen", "preset": …}` (props apply over the preset), in
Object ▸ Modifiers and Add New Effect ▸ Modifiers:
- **Ben-Day Dots** (`benDay`): round dots at 45°, 10 lpi, a fixed 30% tone.
- **Line Fill** (`lineFill`): lines at 45°, 12 lpi, a fixed 30% tone (a hatch).

**Tone sources.**
- `fill`: the darkness (1 − Rec. 709 luminance, times opacity) of the path's paint: its fill, else
  its stroke's paint, else black (solid ink). A linear or radial gradient is read where each point
  sits (its stops, midpoints and opacities; an unplaced gradient fitted to the path's box, as it
  draws). A freeform gradient is read as the average of its colours, and a pattern as 50%, each
  with a note.
- `fixed`: `amount` everywhere.
- `image`: an image object's darkness (straight-alpha RGBA, Rec. 709 luminance, transparent pixels
  count as white) where each point sits, through the image's placement (rotated, skewed and
  flipped images are read where they show), bilinear; no ink outside the image. The image is
  found in the document by id, as a copy-to-shape target is: refused when set if it names nothing,
  the object itself, its groups or contents, or a top-level layer; it follows copies (duplicating
  the screened object with its image points the copy at the copied image); moving or editing the
  image redraws the screen. The file's header is read first: an image claiming more than 64 million
  pixels (16 million in the browser) is refused before anything is decoded. A decoded image is kept
  as one byte a pixel, averaged down to at most 16 million pixels, by the content of its file (so
  two objects showing the same file share it; at most 128 MB kept, 48 MB in the browser); an image
  that can't be read is remembered too, so it isn't tried again every frame. When the image is gone,
  isn't an image, or can't be read, the row is skipped (the path draws as it is) with the reason in
  the panel.

**What it makes.** Each path's outline is screened on its own, with its own paint: paths, each
compound path as one outline (its members become one member holding the dots), and every path in a
group; guides and clipping paths are left alone; type, images, symbols and live objects pass through
with a note. The dots replace the path (a live shape becomes a plain path), with **no stroke**,
filled non-zero. **Their ink** (the lead's decision, PIC20.5d):
- `tone: fill`: **black**. The paint gives the tone only; inking the dots in the paint they read
  would darken twice (a pale gradient edge would show pale dots over little area).
- `tone: fixed` and `tone: image`: the path's fill (its stroke's paint when it has no fill; black
  with neither): a red square with Ben-Day dots gets red dots. A gradient fill stays where it was
  on the outline's box. A **near-white** ink would vanish on white paper, so it inks the dots black
  instead (PIC20.5e): a solid colour whose Rec. 709 luminance is over 0.9 (white, pale greys over
  90%, pale yellow), or a gradient whose every stop is; Ben-Day Dots on a new (white) rectangle
  show black. A light colour under the threshold (grey 88%, a light yellow-orange) keeps its colour.

**Method.** The field is sampled over the path's box (`quality` samples a cell, on a lattice fixed to
the page), contoured by Picabia's marching squares, the rings fitted with curves (Simplify's
least-squares fit within 0.15 of a sample step; round and elliptical dots smooth, the other spots
keep their corners), then clipped to the outline: dots wholly inside are kept, dots wholly outside
dropped, and only the dots that cross the outline's edge (or enclose it) go through Pathfinder's
intersect. Measured: a fixed tone inks its share of a square within 3 percentage points for every
spot (a cross under 20% needs a quality of about 48: its arms are thinner than a sample at 12).

**Copies.** The screen is fixed to the page, not to the object: its cells sit at whole periods from
the page origin. Below a copy row each copy is screened where it sits, so the copies' dots line up as
one screen (copies a whole number of cells apart get the same dots, moved); a rotated copy is not
screened in its own frame. Above every copy row the object is screened once and every copy is a copy
of the dots.

**Caps.** The sampled grid stays within 1,000,000 cells: a finer step is coarsened, with a note.
Before tracing, the anchors are estimated as 4 per screen cell over the path's box and checked against
the object's share of the 200 000-anchor geometry cap (each copy its share); after fitting and
clipping, the real count is checked again. Over either, the path is left as it is, with a note (lower
the `lpi` or shrink the art). The clip is bounded too, each limit leaving the path as it is with a
note: an outline of more than 20 000 segments, or flattened into more than 200 000 pieces; band
slots for the pieces at most 2 000 000 (fewer, taller bands when a detailed edge would fill more);
the crossing and inside tests counted before they run (at most 50 million piece visits); the edge
dots touching more than 20 000 outline pieces in all (a comb-like edge each dot would cross many
times). A boolean that fails at the edge drops those edge dots, with a note.

**Stencil and laser.** The stencil and `laser.gcode` read the screened copies like any geometry row:
each dot is a contour to cut (tested: a 30% screen above a stencil leaves each colour 30% of its
area; the laser burns the dots' perimeters).

#### What geometry rows change

- Paths, the members of compound paths and the paths inside groups. A live shape (rectangle,
  ellipse, polygon, star) becomes a plain path in the copy. Guides are left alone.
- Type, images, symbol instances and live objects (blends, envelopes, meshes, repeats…) pass
  through unchanged, with a note in the panel.
- **Cap:** the anchors the rows make for one object, all its copies together, stay within 200 000
  (`MAX_GEOMETRY_ANCHORS`): each copy may hold its share (200 000 ÷ copies). A row that would make
  more, and more than it was given, is skipped for that copy, with a note; so is a row that gives a
  point that isn't finite. Trim, Tidy and Displace never make more anchors than they are given (a
  cut adds at most two, a displace none), so they are never skipped; nor is Deform, which fits its
  pieces in the copy's share (longer pieces when it must). Nesting doesn't multiply the cap: when the members of a
  copied object have geometry rows whose anchors, times the object's copies, pass 200 000, the
  members draw without their modifiers in each copy (with a note). `modifier.apply` returns what a
  row couldn't do as `warnings` (a skipped row's reshape isn't in the baked copies). Catmull-Rom and
  Chaikin leave alone a subpath with too few distinct points (an open one needs 2, a closed one 3).
- The pivot of the copy rows stays the centre of the object's own bounds, even when a row above
  them reshapes it.


## Screen: contouring core, written for Picabia; not derived from the AGPL marching-squares package

Screen (PIC20.5) is a vector halftone, like a bitmap halftone screen followed by tracing. Its core
landed in PIC20.5a; the `screen` geometry row (PIC20.5c, [screen](#screen) above) runs it.

- **Contouring** (`vectorcraft_modifiers::contour`): marching squares written from the textbook
  algorithm. tlprocedural uses the `marching-squares` npm package, which is AGPL; its source was not
  read, used or ported (nor any other GPL/AGPL code). `contours(&Grid, level)` gives closed rings
  round where the field is `>= level`; `bands(&Grid, lo, hi)` round `lo <= field <= hi`. The 16-case
  table runs every segment with the inside on its right, so outer rings are clockwise on screen and
  holes counter-clockwise: even-odd and non-zero fill agree. Saddles are decided by the cell
  centre's average. The grid is framed by outside samples, so a region reaching the edge closes
  along it. NaN and ±∞ samples are outside. Grids hold at most 1,000,000 cells; bigger, smaller
  than 2 × 2, or with a bad step, origin or level are refused with an error. Perf: a 1000 × 1000-cell
  grid contours in about 6 ms (release build, Apple silicon; sampling a sine field about 3 ms).
- **Field** (`vectorcraft_modifiers::screen`): `Screen::new(shape, lpi, angle, quality)` with
  shapes Round, Ellipse, Line, Square, Diamond and Cross; cells `72 / lpi` pt apart (lpi
  1–300, default 20) turned by the angle (clockwise on screen); quality is samples per cell
  (4–64, default 12). Tone (0 no ink … 1 solid) comes from any function of the point: a
  `ToneImage` (an image's darkness, from 8-bit grey or RGBA, bilinear, no ink outside it or where
  transparent; at most 16 M pixels), the layer's fill, or a fixed share. Each shape's threshold
  comes from its own table of spot values, so the inked share of a cell equals the tone (within
  2% traced). Round, Square and Diamond dots meet at mid-grey and turn into holes in the shadows.
  Art too big for the cell cap at the chosen quality is sampled more coarsely (`Traced::coarsened`).
  Ben-Day dots are Round at a fixed tone; Line Fill is Line at a fixed tone.
- **Wired** (PIC20.5c): the row clips the rings to each path's outline (Pathfinder's intersect, edge
  dots only), fits curves to them (Simplify) and paints them; see [screen](#screen).

## Expressions

The amounts of `transformEach` (`offsetX`, `offsetY`, `rotate`, `scaleX`, `scaleY`) and
`randomize` (`position`, `rotation`, `scale`) and the fill shifts of `color` (`hue`, `chroma`,
`lightness`) can each be driven by a per-copy expression, stored as
`<field>Expr` next to the field (`"rotateExpr": "i * 15"`) and set with
`modifier.set {modifier, props: {"rotateExpr": "i * 15"}}` (an empty text or `null` clears it).
`modifier.set` refuses texts that don't parse (the error says where).

| Name | Meaning |
|---|---|
| `i` | The copy's index (0 = the original) |
| `n` | The number of copies |
| `t` | `i / (n - 1)`, 0 when `n = 1` |
| `value` | The field's own value |
| `pi` | 3.14159… |

Operators, loosest first: `+ -`, `* / %`, unary `-`, `^` (power, right-associative). Functions:
`sin cos tan` (degrees, like every angle in Picabia), `asin acos atan atan2(y, x)` (degrees),
`abs floor ceil round sqrt sign exp ln`, `min max` (1 to 8 arguments), `mod(a, b)`,
`clamp(x, lo, hi)`, `lerp(a, b, t)`, `rad`, `deg`, and `rand(k)` (a fixed number in 0 to 1 for each
`k`, the same on every platform).

| Expression | Effect |
|---|---|
| `rotate = i * 15` | 0°, 15°, 30°… |
| `offsetY = sin(t * 360) * 40` | One wave across the set |
| `scaleX = value * (1 - t / 2)` | Shrinks to half the field's scale |
| `rotation = value * t` (randomize) | No randomness on the first copy, full on the last |

An expression gives that copy's full amount: it replaces the `progressive` ramp for its field
(`t` is there to ramp), while `jitter` still applies. Results are clamped to the field's range, and a
result that isn't a number (`1 / 0`) falls back to the field's value. Expressions are at most 256
characters and 32 levels deep, so they evaluate in bounded time; a damaged one in a file is ignored
with a warning.

The same language drives **field bindings** ([`parameters.md`](parameters.md)), with two
differences: there `t` is the clock (0 for now) and any other name is a document parameter. A field
bound with `modifier.bind` ignores its own `<field>Expr`.

## Canvas guides and handles (PIC5.2 to PIC5.4)

With the **Selection tool** and one editable object selected (not locked or hidden, no anchors
picked on their own), the rows open in the Appearance panel show their handles on the canvas,
each with its guides (dashed, in the selection colour). A row is open when its settings are shown
(the disclosure) or it is the row last clicked; several open rows show all their handles. With no
row open, the **last enabled row that has handles** shows them, as in picabia-craft. Other tools,
a selection of several objects, and turned-off rows show none.

The handles are round knobs ringed in the selection colour, larger under the pointer, a fixed size
on screen at any zoom. A press within 7 screen pixels takes the nearest (before the Selection tool
sees the press, so a knob over the object wins over clicking the object; the bounding box's own
handles win where they overlap a knob; where two knobs coincide the later row's wins, and within a row the one listed first in the table below, so a ring of radius 0 drags its radius out and a trim window over the whole closed path drags its end). Alt, Cmd and Ctrl presses stay the tool's; a Shift-press on a knob is the knob's (for its Shift constraint), so Shift-click doesn't deselect there. A drag shows a
readout beside the knob, previews `modifier.set` and is **one undo step** ("Set Modifier"); a drag
that ends where it began leaves none, and **Esc** puts the params back. A drag whose document is switched, whose object is deleted or whose interaction another command ends (an agent mid-drag) is dropped, its own open preview rolled back. The open rows are kept per document. The handles of one document version, selection, zoom and open rows are built once and shared by the hover, cursor and drawing. The math is in document
points, so a zoomed or turned view (Rotate View) drags the same values; the cursor shows how the
knob moves (four-way, a resize arrow turned with the view, or the rotate cursor).

A handle whose field is **bound** to a parameter or an expression ([`parameters.md`](parameters.md))
sits where the bound value puts it but is drawn greyed, shows the not-allowed cursor, doesn't drag,
and a press says in the status bar to change it in the Appearance panel. The panel stays the full,
keyboard-reachable way to set every field; `modifier.handles` lists the handles shown (for agents
and tests). Knobs that would sit on the bounding box's handles at the default params are placed
18 screen pixels outside the box instead.

| Row | Handle | Drag sets | Shift |
|---|---|---|---|
| linearArray | the last copy's centre (count > 1) | `offsetX`, `offsetY` (`count` stays) | 45° steps |
| circularArray | the ring's centre | `centerX`, `centerY` (the whole ring moves) | one axis |
| circularArray | a knob on the ring opposite the original | `radius` (the original stays; the ring grows away from it) | |
| circularArray | a knob just outside the ring where the sweep ends | `sweep` (−360…360; it follows the pointer round, more than half a turn) | 15° steps |
| gridArray | the left edge of the last column, the top of the last row (more than one) | `gapX` / `gapY` | both gaps equal |
| mirror | a knob at the end of each axis line | `offset` (the axis moves across) | |
| wallpaper | the symmetry origin (not p1, which has none) | `originX`, `originY` | one axis |
| wallpaper | the end of the cell's first side | `cellWidth` | |
| wallpaper | the end of the second side (oblique, rectangular and centred groups) | `cellHeight`; for p1 and p2 also `angle` | keeps the angle |
| deform taper | knobs past each end of the axis, at the scale's height | `startScale`, `endScale` | |
| deform twist | a knob past the end of the axis, turned by half the angle | `angle` (follows the pointer round, past ±360° to ±720°) | 15° steps |
| deform bend | the bend point on the axis | `bendPoint` | |
| deform bulge | a knob on the reach, up and to the right of the centre | `radius` | |
| deform shear | a knob above (beside) the box, slanted with it | `shearX` (`shearY`) | |
| trim | the window's start and end on the object's first subpath | `start`, `end` (the point of the path nearest the pointer) | |
| displace | a knob above the object, its height the amplitude | `amplitude` | |

Every value is clamped to its field's range; a drag to a point that isn't finite sets nothing.
Geometry rows' handles sit on the object itself, also when the row runs on each copy (below a copy
row): copy 0 is the object.

**Not built** (picabia-craft had none either, or the param doesn't exist): transform each,
randomize, copy to shape, resample, tidy, screen and the stencil have no handles; the mirror has
no angle (its `axis` is vertical, horizontal or both, so there is no angle to drag); deform's
`bendAngle`, `strength` (bulge) and the axis itself (horizontal or vertical) are set in the panel;
displace's other fields too. Handles don't snap to other objects (picabia-craft's snapped to
layers' edges and centres), the readout is English and in points whatever the General unit, and
a Trim handle measures the first subpath only (the row cuts every subpath by the same fractions).

## Laser: who may do what

Picabia drives a GRBL laser (`laser.*`, the Laser window: File ▸ Laser…). The rules, from
picabia-craft's `LASER_CONTROL.md` and the safety review of the port:

| Who | Simulator (`port: "sim"`) | Real machine |
|---|---|---|
| A person clicking in the desktop app's Laser window | everything | everything; Start and Resume only after the safety checklist |
| Scripts, agents, MCP, the control channel, `command.batch`, plug-ins, accessibility clients | everything (injected clicks may arm the simulator, for tests and screenshots) | connect, disconnect, status, ports, exports (`laser.gcode`, the stencil exports), profile, `laser.jogCancel`, pause and stop only: `laser.home`, `laser.jog`, `laser.unlock`, `laser.setOrigin` and `laser.frame` are refused, and so are `laser.start` and `laser.resume` (they need the arm) |

- **Filter plug-ins calling commands** (`vc_call`, upstream's review in `cmd/plugin_call.rs`) call
  no `laser.*` or `camera.*` command at all, not even on the simulator: Picabia's commands are
  placed in `cmd/plugin_call/picabia.rs`, where the hardware links are denied.
- **Arming** (`laser.start`, a beam `laser.frame`, `laser.resume`) goes through
  `Session::laser_execute_armed`, the **motion commands** through `Session::laser_execute_person`.
  Neither is a command; the app calls them only from the Laser window (`picabia_laser_window/run.rs`).
- **Input from automation** never counts as a person on a real machine: a frame of control-channel
  input, a control request being answered, or an AccessKit action (any screen reader or
  UI-automation client). Where a press came from is recorded when it goes down (the checklist's
  Start, and the window's motion buttons), not when its click lands; boxes automation ticked are
  cleared and must be ticked again by a person.
- **The job Start runs is the one the checklist showed:** the checklist keeps the dry run's size
  and line count, and confirm works the job out again; if it changed (the design was edited
  meanwhile) Start is refused and the checklist shows the new numbers, its boxes cleared.
- For a second after a motion command is sent (before the status says the machine moves) the laser
  counts as working: no file dialog opens.
- **Real ports** are on only in the desktop app, and off whenever it runs with `--control` (agents,
  MCP through the control channel) unless `PICABIA_LASER_HARDWARE=1`; `PICABIA_LASER_HARDWARE=0`
  turns them off always. Tests, the CLI and headless MCP sessions never turn them on.
- **Stop** is always on screen while connected (the window, or a bar when it is closed); Shift+Esc
  stops from every window of the app (floating panels' own windows too), whenever the link is
  open. Esc alone never stops. No file dialog opens while the laser works (it would cover Stop).
- **Gap:** while a menu of the macOS menu bar is open the app runs no frames, so neither Stop nor
  Shift+Esc acts until it closes: the machine's own emergency stop must be in reach (the checklist
  asks).

