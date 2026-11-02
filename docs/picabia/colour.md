# Colour

**Status:** 2026-11-02, planned, not started (PIC32). This is step 3 of the porting plan (version 3,
"Colour: copies get their own colours"), moved ahead of the ramp chip and the Offset row at the
owner's request: colour is the next modifier work. The plan is written so that a session without
the owner's machine (no `plan/`, no tlprocedural checkout) can build it: every number tlprocedural
uses is copied here.

Ported from the owner's tlprocedural, which the owner wrote and gave permission to port:

- **Color**: `src/store/modifiers/core/ColorModifierProcessor.ts`, `ColorSettings` in
  `src/types/modifiers.ts`, `COLOR_DEFAULTS` in `src/store/modifiers/modifierConfig.ts`.
- **Color Each**: `ColorEachProcessor.ts` (and its test), `ColorEachSettings`,
  `COLOR_EACH_DEFAULTS`.
- The shift maths: `src/components/modifiers/utils/colorShift.ts` (OKLCH through culori,
  `src/components/ui/color-utils.ts`); the ramp: `src/store/modifiers/utils/progressiveScale.ts`
  and `progressiveMultiplier` in `TransformEachProcessor.ts`.

## What it does

One row, **Color** (kind `color`), gives each copy its own colour: it shifts the hue, chroma and
lightness of each copy's fills and strokes, by the same amount on every copy, growing across the
copies (the ramp), or at random per copy (jitter), or all three. Under a Linear Array of 8 it turns
one red square into eight squares running red to violet; under a Grid it scatters slightly different
greens. On an object without copy rows it is a live colour shift of the object itself.

tlprocedural has two modifiers, Color (shift the same way for fill and stroke, ramp along the array
index) and Color Each (separate fill and stroke shifts, sorted by position). Picabia merges them into
the one row, as Deform merged Taper, Bend, Shear and Bulge: the stroke follows the fill or has its
own shifts, and the order the ramp runs in can follow the copies or their position on the page.

## Where it runs

**Color is a per-instance step**, like Transform Each and Randomize (see
[`modifiers.md`](modifiers.md) › Evaluation order): it acts on the **final** copies, after every
array step, wherever its row sits; `i`, `n` and `t` count all the copies; its order among the other
per-instance steps matters only among Color rows (two Color rows apply one after the other: the
second shifts the colour the first made, as tlprocedural chains them, clamps included). It adds no
copies and moves nothing, so bounds, hit testing, culling, the stencil's islands and the laser's
paths are unchanged by it.

- Copy 0 is the original, as everywhere: with the ramp on and `reverse` off, it keeps its colour.
- An object with a Color row and no copy rows draws as one recoloured copy. This is a stack that
  **makes a change** (it must not take the "draws exactly as upstream" shortcut in
  `vectorcraft_render::picabia`, which today keys on copies and geometry rows).
- Geometry rows don't care: reshaping and recolouring are independent.

## Settings

Lenient parse like every row (`crates/modifiers/src/lenient.rs`): missing fields take the default,
numbers out of range are clamped, junk is a default with a note, never an error. Percentages are
tlprocedural's units and mean what they mean there (see The maths).

| Field | Unit | Range | Default | Meaning |
|---|---|---|---|---|
| `hue` | % | -100 to 100 | 10 | Fill hue shift; 100 = a full turn (360°) |
| `chroma` | % | -100 to 100 | 10 | Fill chroma (saturation) shift; 100 = +0.4 OKLCH chroma |
| `lightness` | % | -100 to 100 | 10 | Fill lightness shift; 100 = +1 OKLCH lightness |
| `stroke` | enum | `same`, `own`, `none` | `same` | `same`: strokes take the fill's shifts and jitter (tlprocedural's Color, and Color Each with `strokeLinked: false`). `own`: the stroke fields below. `none`: strokes keep their colour |
| `strokeHue`, `strokeChroma`, `strokeLightness` | % | -100 to 100 | 0 | Stroke shifts when `stroke` is `own` |
| `progressive` | on/off | | on | The ramp: shifts are multiplied by `t × amount` (see below). Off: every copy gets the full shift |
| `amount` | factor | 0 to 1 | 1 | Ramp intensity (tlprocedural's `progressiveAmount`) |
| `reverse` | on/off | | off | Ramp runs the other way (the last copy keeps its colour) |
| `order` | enum | `copies`, `array`, `leftRight`, `topBottom`, `diagonal` | `copies` | The order the ramp and jitter count copies in: copy index across all copies; `array`: the copy number in one array row only (below); or the copy's centre left to right, top to bottom, or by `x + y` (tlprocedural's `sortBy`; its `selection` and `z-index` orders don't apply to copies of one object). Ties keep copy order |
| `array` | integer | 1 to the number of array rows | 1 | With `order: array`: which array row the ramp runs along, counting array rows from the top of the stack (1 = the first). The ramp uses that row's copy number `k` and count `n` (`Instance::stage_indices`), so with Grid then Circular and `array: 2` every grid in the ring gets one step of the ramp, and with `array: 1` each ring repeats the same gradient across its grid (tlprocedural's behaviour). Clamped to the arrays there are; with no array rows it acts as `copies`. Jitter still uses the flat copy index |
| `jitterHue`, `jitterChroma`, `jitterLightness` | % | 0 to 100 | 0 | Random extra shift per copy, plus or minus, added to the fill's shift |
| `strokeJitterHue`, `strokeJitterChroma`, `strokeJitterLightness` | % | 0 to 100 | 0 | Stroke jitter when `stroke` is `own` |
| `seed` | integer | 0 to 4 294 967 295 | 42 | Jitter seed (tlprocedural's default) |
| `hueExpr`, `chromaExpr`, `lightnessExpr` | text | | none | Per-copy expressions for the fill shifts, replacing the ramp for that field (see [`modifiers.md`](modifiers.md) › Expressions) |

Every number field takes a parameter chip like the other rows (`crates/modifiers/src/fields.rs`
lists the bindable fields; PIC6).

Defaults come from tlprocedural's Color (10/10/10, ramp on, amount 1, seed 42), so adding the row
under an array shows a visible gradient at once; Color Each's all-zero defaults would show nothing.

**Presets** (Picabia's own, `modifier.add` `preset`, like Ben-Day Dots):

- **Rainbow**: hue 100, chroma 0, lightness 0, ramp on: one full turn of the hue across the copies.
- **Fade**: hue 0, chroma 0, lightness 60, ramp on: copies fade towards white.
- **Vary**: all shifts 0, ramp off, jitter hue 8, chroma 10, lightness 10: each copy a little
  different.

## The maths

Per copy `k` (counted in `order`), with `n` copies:

1. **Ramp multiplier** `m`: 1 when `progressive` is off. When on: `u = k / (n - 1)` (`1 - u` when
   `reverse`), `m = u × amount`; with `n = 1`, `m = amount` (so the row works on a lone object, as
   both of tlprocedural's rows do at amount 1).
2. **Jitter** per channel: `r × jitter`, where `r` in [-1, 1] comes from SplitMix64 of `seed`, the
   copy's index and the channel (fill hue, fill chroma, fill lightness, stroke hue, …), the same
   way Randomize draws (`crates/modifiers/src/kinds/rng.rs`). A copy's jitter never changes when
   copies are added after it or when other channels are turned on. (tlprocedural draws from one
   mulberry32 stream in order and has to burn slots to keep channels aligned; this is a deliberate
   difference, listed below.)
3. **Shift** for each channel: `s = field × m + jitter` (an expression replaces `field × m`).
4. **Apply** to each colour, in OKLCH (OKLab in polar form, from sRGB, VectorCraft's own OKLab code
   in `crates/color/src/libraries.rs`, today `pub(crate)`: make it public, don't copy it):
   - `L = clamp(L + s_l / 100, 0, 1)`
   - `C = clamp(C + s_c / 100 × 0.4, 0, 0.4)`
   - `H = (H + s_h / 100 × 360) mod 360`, and when `s_h ≠ 0` and `C < 0.02` (white, greys, black),
     `C = min(0.4, |s_h / 100| × 0.2)` so a hue shift shows on a neutral (tlprocedural does this;
     keep it, it's why Rainbow works on the default white rectangle).
   - Back to sRGB, **keeping the colour in gamut by reducing chroma** (owner's decision,
     2026-11-02): if the result falls outside sRGB, keep `L` and `H` and lower `C` by bisection
     (a fixed 20 steps, so the result is the same bits natively and in wasm) to the largest chroma
     whose linear sRGB channels all lie in [0, 1] (a tolerance of 1e-6); then clamp each channel to
     [0, 1] to remove rounding. tlprocedural clamps each channel instead (culori's `formatHex`),
     which can turn the hue of very saturated results; this is a deliberate difference.
   - A shift of 0 on all three channels leaves the colour bit-for-bit as it was (no round trip).

Golden values for tests: compute them once from the formulas above with a tiny reference (the
crate's own test, not tlprocedural's code), including red with hue 50 (half a turn), white with hue
10 (chroma injected), black with lightness -10 (clamped at 0), and chroma 100 on a saturated blue
(clamped at 0.4, then chroma reduced into sRGB with the hue kept: test that the result's OKLCH hue
is within 0.5° of the input's).

## Which colours change

Each copy is recoloured with `vectorcraft_doc::recolor::recolor_node` (the visitor Edit Colors,
Recolor Artwork and the colour adjustment effects share) with `Reach { fill, stroke, images: false }`
and a visitor that runs the fill shift on fills and the stroke shift on strokes:

- Solid fills and strokes, text runs' fills and strokes, every gradient stop (linear, radial,
  freeform points), gradient-mesh points and a colorized image's colour.
- **Swatches.** A shifted colour loses its swatch link in that copy (`map_paint` does this); the
  swatch itself and the original object are untouched. A channel whose shift is 0 keeps the link.
- **Colour models.** RGB colours stay RGB. CMYK, Gray, Lab and spot colours are converted to sRGB
  through the document's colour conversion, shifted, and converted back to their own model (a spot
  colour becomes a process colour of the document's mode). A Gray colour that gains chroma becomes
  RGB (or CMYK in a CMYK document). Alpha and the paint's opacity never change.
- **Left alone** (owner's decisions, 2026-11-02): None, Registration, pattern swatches' tiles (a
  note under the row when the object has one: "pattern fills keep their colours"), embedded and
  linked images' pixels, and effect colours (drop shadow, glows, Inner Glow).
- Members of a group or compound shape all get their copy's shift (one shift per copy, not per
  member; per-member colour is PIC32.6).

## Drawing, Apply, exports

- The stack's output carries, per instance, the list of colour shifts to apply (empty for a stack
  without Color rows): add it to `vectorcraft_modifiers::eval::Instance` (or a parallel vector in
  `Output`, whichever keeps the hot path for arrays without colour free). The crate stays pure: it
  computes the shift numbers; the recolouring happens where the copy is built.
- `vectorcraft_render::picabia::make` recolours each copy after its transform and the geometry rows
  (one place, so the canvas, every export, `modifier.apply`, Package and the copy-as-PNG path all
  agree, as for every other row). The art cache already keys on the evaluated output, so a changed
  shift redraws.
- `modifier.apply` bakes the recoloured copies (shifted colours become the copies' own paints).
- Vector exports write the shifted colours as plain paints; the live row survives where the stack
  does (`.vectorcraft`, SVG and PDF with Preserve Editing).
- **Stencil**: the stencil reads colours as drawn, so a Color row above a stencil changes which
  colour layer a copy falls in. Test that it does, and that a stack without Color rows gives the
  same sheet as before.
- **Laser**: unchanged (it cuts paths).
- **Budget**: recolouring 10 000 copies of a simple path must stay within the existing render
  budget (`vectorcraft-cli perf`); convert each distinct base colour once per row and cache the
  shifted result by `(colour, shift)` while building one art.

## Commands and UI

- `modifier.add.color` (Object ▸ Modifiers and the palette, "Add Color"), the presets
  `modifier.add.rainbow`, `modifier.add.fade`, `modifier.add.vary` as `benDay` and `lineFill` are
  done (`crates/engine/src/cmd/picabia_modifier/add_kinds.rs`).
- The Add Modifier menu gets a **Color** group between Shape and Make: Color, Rainbow, Fade, Vary
  (`crates/ui-egui/src/picabia/add_menu.rs`; the group comment says what the groups mean, add
  "Color recolours the copies").
- The row's fields in the Appearance and Properties rows (`crates/ui-egui/src/picabia/appearance.rs`),
  grouped Fill (hue, chroma, lightness), Stroke (the mode, and its three fields when `own`), Ramp
  (on/off, amount, reverse, order), Jitter (three or six fields, seed). The row's title shows a
  small strip of the first, middle and last copy's fill colour (drawn in code from the evaluated
  shifts; no asset).
- Canvas handles: none in phase 1.

## Deliberate differences from tlprocedural

- One row instead of Color and Color Each.
- `stroke: same | own | none` instead of `strokeLinked` (whose `true` meant independent).
- Jitter per copy and channel from SplitMix64 of seed, index and channel, so copies are stable as
  the count changes and match natively and in wasm; tlprocedural uses one sequential mulberry32
  stream.
- The ramp counts the final copies by default (`t` as in [`modifiers.md`](modifiers.md));
  tlprocedural's Color ramps along one array's index, which is `order: array` here, with the array
  chosen.
- Out-of-gamut colours keep their hue (chroma reduced), where tlprocedural clamps each channel.
- Multi-selection (`sourceShapeIds`) isn't ported: a Picabia stack lives on one object; group the
  objects instead (and PIC32.6 colours members one by one).
- Colours in VectorCraft are not only sRGB hex: the model rules above are new.

## Tasks

One task id per commit; `cargo xtask ci` green before each.

| Task | What |
|---|---|
| PIC32.1 | The row in `vectorcraft-modifiers`: model (`model_color.rs` beside `model_deform.rs`), lenient parse and round trip, the ramp, order, jitter, expressions, bindable fields; per-instance shifts in the output; the OKLCH maths with golden tests (in `vectorcraft-color`, OKLab made public there; layers allow modifiers → color, check `cargo xtask layers`); native vs wasm32 identical shifts for a 200-copy stack |
| PIC32.2 | Drawing: recolour in `picabia::make` (fills, strokes, text, gradients, mesh, colorized images, swatch links, colour models), the "makes a change" rule, `modifier.apply`, exports, art cache, the perf budget; render pixel tests and an SVG export test |
| PIC32.3 | Engine: `modifier.add.color` and the three presets, params docs, the junk sweep (`engine/tests/picabia_modifier_sweep.rs`, `command_sweep.rs`), the stencil interplay test |
| PIC32.4 | UI: the Color group in Add Modifier, the row's fields and colour strip in Appearance and Properties; headless egui frame tests (as `panels/transparency.rs`) and pictures with `vectorcraft-cli run … --export x.png` (no screen in a cloud session) |
| PIC32.5 | Docs: a `color` section in [`modifiers.md`](modifiers.md) (Parameters, and the per-instance steps list in Evaluation order), the PIC32 rows and the P-step 3 line in [`ROADMAP.md`](ROADMAP.md), this page's status |
| PIC32.6 | Later: `each: copies | members`: in a group, each member gets its own step in the ramp (tlprocedural's Color Each on a group) |
| PIC32.7 | Later, separate: tlprocedural's OKLCH picker in the Color panel and the fill/stroke popovers (upstream panels: needs seams listed in [`SEAMS.md`](SEAMS.md); the decision is recorded in the seam map) |

## Working beside physics (PIC31)

A cloud session is building physics phase 2 at the same time. To keep merges small:

- **Colour owns** `crates/modifiers`, `crates/render/src/picabia.rs` (and its tests), the colour
  maths in `crates/color`, `crates/engine/src/cmd/picabia_modifier/*`, and
  `crates/ui-egui/src/picabia/{add_menu,appearance}.rs`.
- **Physics owns** `crates/physics`, the physics commands and panel row, the transport bar and the
  Selection tool's poke. Don't edit them.
- Shared files: [`ROADMAP.md`](ROADMAP.md) (add PIC32 rows below PIC31's; don't touch its row) and
  `add_menu.rs` (physics may change the Make group: put Color as its own group above Make). Rebase
  on `origin/main` before each push; the second to land resolves.
- Use a separate `CARGO_TARGET_DIR` and delete it when done.

## Decisions (owner, 2026-11-02)

The owner answered these before the build and won't be watching it; the implementing session
follows them without asking again:

- **Effect colours** (drop shadow, glows, Inner Glow) are left alone.
- **Out of gamut:** reduce chroma until the colour fits, keeping hue and lightness (The maths, step 4).
- **Pattern fills** keep their tiles' colours, with a note under the row.
- **Nested arrays:** the ramp runs across all copies by default, and `order: array` with `array`
  picks one array row to ramp along (Settings).

Anything else the spec doesn't settle: choose what the best expert would, write the choice and why
into this page and the commit message, and carry on.

## Revision history

- 2026-11-02: planned (PIC32), from tlprocedural's Color and Color Each and the porting plan's step 3.
- 2026-11-02: the owner's decisions on effect colours, gamut, patterns and nested arrays; `order:
  array` and `array` added.
