# Modifiers

> The modifier spec, ported from picabia-craft (`docs/picabia/modifiers.md` there, params as of its
> P8) and rewritten for VectorCraft. The stack crate (`crates/modifiers`, PIC1.1) follows this page;
> drawing, the panel rows, Apply and the handles are not ported yet (see [`ROADMAP.md`](ROADMAP.md)).

A **modifier stack** is a list of live, non-destructive steps attached to an object. The object keeps
its own geometry; the stack describes how it is repeated and transformed. Edit the object or any
modifier and every copy follows. Paths, compound shapes, text and groups can carry a stack. Copies
are drawn, exported and hit-tested as part of their source object: clicking a copy selects the
source.

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
  panel, in one ordered list with VectorCraft's own effects, built from VectorCraft's widgets (no
  separate Modifiers section). Evaluation rule: **Picabia rows run in their order, top first**, as
  below. A VectorCraft geometry effect **below** a copy row (an array, mirror, wallpaper or copy to
  shape) applies to **each copy**; one **above** it applies to the object **before** it is copied.
  Until the interleaving is built, the simplest correct ordering that keeps this rule is shipped:
  the Picabia rows are one block in the list, between the fills and strokes and the object's own
  effects, and every VectorCraft effect counts as below the copy rows (it draws on each copy; see
  How copies are drawn). A row's eye turns it off, its disclosure shows its fields, its `⋯` menu
  moves, duplicates, copies, pastes, removes it or applies the stack down to it; under the rows are
  the evaluation's warnings and Apply. Add New Effect (`fx`) ▸ Modifiers and Object ▸ Modifiers
  add each kind.
- **No variable bindings.** picabia-craft's P6 bound modifier fields to FigCraft variables
  (`Node.bound`, `variable.bind`); there is no `bind.rs` here. P6 is replaced by the **parameters**
  design (number and text parameters in VectorCraft's Variables panel, with a parameter, expression
  or ramp chip on every number field). The per-field expressions below (`<field>Expr`) stay in the
  stack format.
- **No generator rows.** picabia-craft's `imageTrace` and `pathfinder` generator rows (its P13, P14)
  are not ported: VectorCraft's own Image Trace and live Pathfinder are used, and a stencil can sit
  on a traced image.

## Evaluation order

**The top row runs first**, like a recipe. Each step acts on everything the step above produced:

- **Array steps** (linear, circular, grid, mirror, wallpaper, copy to shape) repeat the **whole
  previous set as a block**. Grid, then Circular is a ring of grids. Copy 0 of an array is the
  identity: the original is one of the `count` copies (copy to shape doesn't keep the original).
- **Per-instance steps** (transform each, randomize) change each instance around its own pivot and
  do not add copies. They always act on the **final** copies, after every array step has run,
  wherever their row sits in the stack: each copy gets its own values, and `i`/`n` count all the
  copies. (Their position among the arrays doesn't change the result; their order among
  themselves does.)
- A **stencil** step ends the stack (see `stencil` below): it adds no copies, and the enabled steps
  below it are ignored with one warning.
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
- An object whose stack makes no copies (no active row, a single unmoved copy, a missing
  copy-to-shape target) draws exactly as VectorCraft draws it without the stack.

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


### stencil

Turns the layer, with every copy the rows above it made, into a laser-cut **katagami** stencil: one
sheet of paper or film whose cut-outs let resist paste through. It is the last step: it adds no
copies (the copies above it stand), and enabled rows below it are ignored with one warning (only
the first enabled stencil counts). Apply bakes only the rows above it; the stencil row moves onto
the new group, live.

**Status here:** the `stencil` kind is in the ported stack crate (PIC1.1) and is parsed, saved and
evaluated there; the sheet, islands, ties, preview and laser export are being ported (picabia-craft
P8 to P12, P17). The fields below are picabia-craft's and stay the contract.

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


## Expressions

The amounts of `transformEach` (`offsetX`, `offsetY`, `rotate`, `scaleX`, `scaleY`) and
`randomize` (`position`, `rotation`, `scale`) can each be driven by a per-copy expression, stored as
`<field>Expr` next to the field (`"rotateExpr": "i * 15"`) and set with
`modifier.set {modifier, props: {"rotateExpr": "i * 15"}}` (an empty text or `null` clears it).
`modifier.set` refuses texts that don't parse (the error says where).

| Name | Meaning |
|---|---|
| `i` | The copy's index (0 = the original) |
| `n` | The number of copies |
| `t` | `i / (n - 1)`, 0 when `n = 1` |
| `value` | The field's own value (a parameter's value once parameters drive the field) |
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

## Canvas guides

Each array shows guides while its layer is selected: linear, the offset line from the first to the
last copy; circular, the ring and its centre; grid, the outline of the whole lattice; mirror, the
axis line(s). Transform each and randomize show none.

## Canvas handles

With the Selection tool and one unlocked object selected, the **last enabled modifier that has handles**
shows them (the Appearance panel's open row lives in the UI, so it doesn't choose). They look and behave
like VectorCraft's own handles: dots ringed in the selection colour, larger under the pointer, a
readout beside the dragged one, one undo step per drag ("Set modifier"), Esc cancels, and the
dragged point snaps as VectorCraft's Smart Guides and snapping do.

| Modifier | Handle | Drag sets | Shift |
|---|---|---|---|
| mirror | a knob at the end of each axis line | `offset` | |
| circularArray | the ring centre (dot inside) | `centerX`, `centerY` (the whole ring moves) | horizontal or vertical only |
| circularArray | a knob on the ring opposite the original | `radius` (the original stays; the ring grows away from it) | |
| linearArray | the last copy's centre (shown when `count` > 1) | `offsetX`, `offsetY` (`count` stays) | 45° steps |
| gridArray | the middle of the lattice's right / bottom side (more than one column / row) | `gapX` / `gapY` | both gaps equal |
