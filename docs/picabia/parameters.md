# Parameters and bindings (PIC6)

**Status:** 2026-11-01. PIC6.1 to PIC6.3a: parameters, field bindings, their commands and their UI.
This replaces picabia-craft's P6 (FigCraft variable bindings) with the owner's design.

A **parameter** is a named number or text kept with the document. Any **number field** of a
modifier row (the copy rows, the geometry rows and the stencil's settings) can be **bound** to a
parameter or to an **expression**, from the small chip beside the field. Change the parameter, or
pick another row of a VectorCraft data set, and every object bound to it redraws.

## Parameters

Window ▸ Variables has a **Parameters** section between its Variables and Data Sets lists:

- one row per parameter: its name (type to rename; bindings that read it follow), a slider and a
  field for a number (the slider updates the canvas while it is dragged and makes one undo step on
  release), or a text field for a text, and a bin;
- under it, the parameter's **data source**: a VectorCraft variable. While a data set is active and
  holds a value for that variable, the value is the parameter's ("from the data set"; the slider is
  then the data set's, so it is greyed). A number parameter reads the text as a number and clamps it
  to its range; a value that isn't a number keeps the parameter's own value, with a warning under
  the row;
- **+ Number** and **+ Text** add one (`param1`, `param2`…).

So VectorCraft's data sets become rows of parameter values: define a text variable (Window ▸
Variables ▸ New Variable…, or a CSV import), give it a value per data set, choose it as a
parameter's data source, then step through the data sets.

| Field | Number | Text |
|---|---|---|
| `name` | A letter or `_`, then letters, digits or `_`; at most 32; unique ignoring case; not `i`, `n`, `t`, `value`, `pi` or a function name | same |
| `value` | Within `min`–`max`, ±1e9 at most | At most 1 KiB |
| `min`, `max` | Default 0–100, widened to take the first value | — |
| `step` | The slider's step; 0 = any value (default 1) | — |
| `source` | A VectorCraft variable, or none | same |

At most 256 parameters per document. They are stored in the document's plug-in data under
`picabia.parameters` (VectorCraft's `pluginData`, which it keeps unread), so the native file, undo,
Save As and a document copied from another keep them, and plain VectorCraft opens and saves the file
without losing them:

```json
{"v": 1, "params": [{"name": "copies", "type": "number", "value": 6, "min": 1, "max": 24, "step": 1},
                    {"name": "label", "type": "text", "value": "A", "source": "label"}]}
```

A damaged entry (no usable name, a repeated name) is dropped when read; a range read backwards is
swapped and a value outside it clamped.

## Bindings

Every number field of a modifier row has a chip on its right:

| Chip | Means |
|---|---|
| Hollow dot | Not bound: the field's own value |
| Filled dot (accent) | Bound to a parameter |
| Filled square (accent) | Bound to an expression |
| Filled square (warning colour) | The binding fails: the field's own value is used; the tooltip says why |

A bound field shows the value it takes, read-only (or "per copy"): to change its own typed value
(what an expression reads as `value`, and the fallback), unbind it, type, and bind again, or use
`modifier.set`. Click the chip, or Tab to it and
press Enter or Space, to open its popover: pick a **parameter**, type an **expression** (checked
as it is committed; a mistake says where), or **Unbind**. A ramp will join them (it reads "Ramp:
coming later"; a newer file's ramp is kept and ignored with a warning).

A binding is kept in the row's own JSON, under `bind`, keyed by field:

```json
{"id": "m1", "kind": "linearArray", "count": 3,
 "bind": {"count": {"param": "copies"}, "offsetX": {"expr": "gap * 2 + value"}}}
```

so it follows its row: moving the row, duplicating it, copying and pasting it, duplicating or
pasting the object keep it; removing the row drops it. Nothing needs remapping: rows keep their ids
and parameters are named.

### Evaluation

Bindings are resolved while the stack is evaluated, never written back:

- the field keeps its **typed value**, which an expression reads as `value`, and which is used
  whenever the binding fails (a parameter that's gone or holds a text that isn't a number, an
  expression that doesn't parse or gives no number such as `1 / 0` or `sqrt(-1)`); the failure is a
  warning on the chip, in the panel's notes and in `modifier.list`;
- results are clamped to the field's range (the panel's) and rounded for whole-number fields;
- an expression reads `i` and `n` (below), `t` (the **clock**: 0 until Picabia has one), `value`,
  `pi` and parameter names; its syntax and functions are those of [`modifiers.md`](modifiers.md) ›
  Expressions. Expressions are at most 256 characters and 32 levels deep. The trigonometric,
  `exp`, `ln` and power functions are the platform's maths library, so a native build and the
  browser build may very rarely differ in the last bit of a result (`rand(k)` is exact everywhere).

**Per copy or once.** `transformEach`'s amounts (`offsetX`, `offsetY`, `rotate`, `scaleX`, `scaleY`)
and `randomize`'s (`position`, `rotation`, `scale`) run on every final copy, so an expression bound to
one of them that reads `i` or `n` gives each copy its own value: `i` is the copy (0 = the original),
`n` all the copies. It replaces the field's progressive ramp (as a per-copy expression does), and a
copy it gives no number for uses the field's value (the warning counts them). Every other field takes
one value per evaluation: there `i` is 0 and `n` is how many copies the rows above made (1 at the
top). That includes the arrays' counts and offsets, mirror, wallpaper, copy to shape, the geometry
rows (deform and displace ramp across copies with their own progressive settings) and the stencil.
For a ramp from 0 to 1 across the copies write `i / (n - 1)` (`t` is the clock here, unlike in the
older per-copy expressions, where it still means that ramp).

**Redrawing.** The evaluation cache key holds the values of exactly the parameters a stack's
bindings read, so changing a parameter re-evaluates and repaints the objects that read it and no
others (the renderer treats bound objects as it treats copy-to-shape sources: no caching by the
object alone, and a parameter change counts in the dirty area). A stencil's cheap key holds every
parameter value (any change re-gathers; its geometry key spares the compute when the cut is the
same). The resolved parameters are built once per version of the document's parameters and
variables (per thread), so a frame with many bound objects resolves them once.

**Symbols.** A bound object inside a symbol's art reads the document's parameters in every instance.
A rename rewrites the art's bindings too, `param.remove` counts its objects, and a change of a
parameter it reads repaints the layers (a symbol's instances aren't tracked one by one).

**Exports bake values.** SVG, PDF, the other vector exports, `modifier.apply` (and Apply Up to Here,
whose later rows keep their bindings on the new group), the stencil and laser export use the values
the parameters have now. The native file keeps the bindings and the parameters live.

### Edges

- A field whose binding names a parameter that's removed uses its own value until a parameter of that
  name exists again (`param.remove` reports the objects still reading it; undo brings it back).
- Renaming a parameter onto a name that dangling bindings still read makes them read it: the reply
  (and the panel's status bar) warns which objects. A `param.set` that changes nothing records no
  undo step; a slider drag is one.
- `modifier.set` on a bound field changes its typed value (what `value` reads and the fallback); the
  binding stays.
- Pasting into another document carries the bindings but not the parameters: they warn until the
  parameters are added there.
- The stencil's `tieAngle` (optional) and fields that aren't numbers (on/off, choices, the
  copy-to-shape target) can't be bound.

## Commands

| Command | Params | Result |
|---|---|---|
| `param.add` | `{name, type?: number \| text, value?, min?, max?, step?, source?}` | `{parameter}` |
| `param.set` | `{name, value?, min?, max?, step?, source? (null: none), rename?}` | `{parameter}` |
| `param.remove` | `{name}` | `{removed, stillBound: [object ids]}` |
| `param.list` (query) | `{}` | `{parameters: [{name, type, value, min, max, step, source, effective, fromDataSet, warning?, objects}], dataSet, clock: 0}` |
| `modifier.bind` | `{ids?\|id?, modifier?\|index?, field, binding: {param: name} \| {expr: text}}` | `{perCopy, bound: [{id, modifier}]}` |
| `modifier.unbind` | `{ids?\|id?, modifier?\|index?, field?}` (no field: all of the row's) | `{removed}` |

Each edit is one undo step. `modifier.list` reports each row's `bind` and, under `bound`, what
every bound field resolved to (`{modifier, field, value, perCopy, error}`). `modifier.bind` refuses a
field that isn't one of the row's number fields, a parameter that doesn't exist or isn't a number,
an expression that doesn't parse with the current parameters, and `{ramp}`. Like every command they
reach the control channel and MCP through the command registry.

## Tests

- `crates/modifiers/src/bind_tests.rs`: binding expressions (parameters, the clock, case), names,
  renames, parameter validation and lenient storage, data sets, a bound count (clamped, rounded,
  missing parameter), once-evaluated fields (`n` from the rows above), per-copy expressions,
  bindings over the field's own expression, randomize, failures (each kind) and per-copy failure
  counts, geometry and stencil rows, round trip and row moves, `bind_field` checks, the cache key
  (an unread parameter keeps the evaluation, a read one changes it), a junk sweep over every kind's
  fields.
- `crates/render/src/picabia_tests.rs`: a parameter change repaints the bound object only, and a
  renderer that drew before draws the new copies.
- `crates/engine/src/cmd/picabia_modifier/params_tests.rs`: the commands with undo, refusals,
  per-copy expressions and rename, rows moved and duplicated, an object duplicated, rows pasted,
  save and reopen, SVG and Apply baking, a data set driving a parameter, params docs, junk.
- `crates/engine/tests/picabia_modifier_sweep.rs`: stacks with bound fields under every `modifier.*`
  and `param.*` command with junk params, and undo of bind, unbind and the `param.*` edits.
- `crates/ui-egui/src/picabia/bind_chip_tests.rs`: the chip's states and tooltips, a bound field's
  read-only value and a focusable chip on every number field, the popover binding an expression and
  unbinding, a failed binding's warning, the stencil settings' chips, the panel's number fields
  matching the bindable fields and their ranges, the Variables panel's Parameters section (slider,
  + Number, typing a value redraws the bound object).

## Gaps

- A symbol whose art reads a parameter repaints the whole layers, not just its instances, when it changes.
- No ramps yet (the chip's popover has a disabled placeholder; the format keeps other binding forms).
- No clock: `t` is 0. Motors, the randomizer and batch export wait for it.
- Text parameters only drive number fields (as numbers); choices and on/off fields can't be bound.
- Parameters don't travel with pasted objects to another document.
- The older per-copy `<field>Expr` params aren't shown in the panel; their `t` keeps its old meaning.
