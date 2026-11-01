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
