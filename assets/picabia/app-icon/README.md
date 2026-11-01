# Picabia app icon

A **ring of eight dots**: one yellow original and seven orange copies around it, the picture of a
circular-array modifier. A placeholder, drawn from simple geometry. It was made for Picabia's first
fork (picabia-craft, commit PIC0.3 `bbe24228`) and is copied here unchanged.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Night | `#1d1b3a` | the full-bleed field |
| Copy orange | `#ff5a36` | the seven copies |
| Original yellow | `#ffd23f` | the original (top) |

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square `rx=112`; eight circles of radius 52 on a
ring of radius 150 around the centre. The macOS render pads it onto Apple's 824/1024 icon grid.

## Provenance

Original work by the Picabia contributors, MIT OR Apache-2.0 (see `LICENSE.txt`). No third-party
material. VectorCraft's dragon icon (`assets/app-icon/`) and the ArtCraft logos (`docs/brand/`) are
not used by Picabia's window or About box; they stay in the tree untouched because they are
upstream's.

## Files

| File | What | Used by |
|---|---|---|
| `picabia.svg` | master vector | source for the PNGs |
| `picabia-macos-512.png` | tile on Apple's icon grid (transparent margin), 512 px | runtime Dock icon on macOS (`apps/vectorcraft/src/titlebar.rs`) |
| `picabia-256.png` | full-bleed tile, 256 px | runtime window / taskbar icon on Windows and Linux (`apps/vectorcraft/src/titlebar.rs`) |
| `picabia-128.png` | full-bleed tile, 128 px | brand mark in the app bar and the About box (`crates/ui-egui/src/brand.rs`) |

The PNGs were rendered from the SVG in picabia-craft (`cargo xtask icon` there, with `resvg`).

## Not yet swapped (packaging)

The bundled icons VectorCraft's packaging uses (`vectorcraft.icns` in the macOS `.app`,
`vectorcraft.ico` embedded in the Windows `.exe`, the Linux `hicolor/` theme icons and `.desktop`
file) are still VectorCraft's: see the P0 gaps in `docs/picabia/ROADMAP.md`.
