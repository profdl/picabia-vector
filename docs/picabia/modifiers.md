# Picabia modifiers in VectorCraft

The spec of the stack (kinds, params, caps) is `crates/modifiers/src/lib.rs`; this page records how
VectorCraft draws and writes it. Revision: 2026-11-01 (A4).

## How copies are drawn

An object with an active stack draws as its **art**: a group that takes the object's place and
identity and holds one copy per modifier instance (`vectorcraft_render::picabia::art`). Each copy is
the object with its geometry transformed by the instance transform, as `modifier.apply` bakes it:
strokes, effects, live corners and pattern tiles scale with the copy.

**Evaluation order.** Picabia rows run in their order and make the copies from the object's
geometry; then each copy draws with the object's fills, strokes and effects (geometry effects and
raster effects such as a drop shadow apply to each copy). The object's transparency (opacity,
blending mode, isolation, opacity mask) applies once, to all the copies together. Members of a
copied group keep their own stacks, within one budget (`MAX_NESTED_COPIES`; past it the members draw
once in each copy).

An object whose stack makes no copies (no active row, a single unmoved copy, a missing copy-to-shape
target) draws as itself.

## Exports

Every vector export (SVG, PDF, print, EPS, DXF, EMF, WMF, legacy .ai, plug-in exporters) writes the
copies as plain objects, with a warning; raster exports draw them. **The written copies carry no
stack**, so reading such a file back gives the copies as plain objects, never copies of copies.
The live stack survives only where the format carries VectorCraft's own document: the native
`.vectorcraft` file, SVG and PDF saved with Preserve Editing, and an EPS that VectorCraft wrote.
(picabia-craft's FigCraft SVG never carried plugin data, so it had no such choice to make.)
