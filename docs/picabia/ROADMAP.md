# Picabia roadmap

**Status:** 2026-11-01. Picabia moved from FigCraft ([picabia-craft](https://github.com/profdl/picabia-craft),
P0 to P17, frozen once ported) to VectorCraft. P0 (identity) and the stack crate are in; every other
phase is being ported.

Picabia is a thin fork of [VectorCraft](https://github.com/storytold/vectorcraft): everything
VectorCraft does, plus a live, non-destructive **modifier stack** (arrays, mirrors, per-copy
transforms, randomisation, wallpaper groups, copy to shape) and a laser-cut **katagami stencil** with
laser export and a GRBL laser controller. New code lives in new crates and files; edits to
VectorCraft's files are short seams marked `PICABIA:` and listed in [`SEAMS.md`](SEAMS.md)
(`rg "PICABIA:"` finds them). Crate names, the `.vectorcraft` file format and the licence (MIT OR
Apache-2.0) are unchanged.

Grades are by behaviour, erring low (A: done and checked in the app; B: works, gaps listed; C:
partial; —: not yet ported). picabia-craft's grades don't carry over: each phase is graded again
once it works here. The gaps listed are picabia-craft's, carried over until the port shows whether
they still apply.

| Phase | What | Grade | Status |
|---|---|---|---|
| P0 | Rebrand (name, id, settings folder, About, icon), CI guards, spec and docs | B | Ported (PIC0.0 to PIC0.6). Name "Picabia" (window title, app bar, macOS app menu title, About), app id `org.picabia.Picabia`, its own settings folder (`Picabia`; portable `PicabiaData`; DrawCraft settings never read), About window crediting VectorCraft, placeholder icon at runtime and in the brand mark, CI guards, README header and these docs. Gaps below |
| P1 | Stack crate (model, parse with caps, cached evaluate) and `modifier.*` commands | C | Ported, crate only (PIC1.1): `crates/modifiers`, stored as the `picabia.modifiers` foreign attribute. Commands not ported. picabia-craft gaps: none open |
| P2 | Draw copies, bounds, hit testing, exports | — | Not yet ported. picabia-craft gaps: nested copies aren't hit-tested per copy; backdrop blur on copies in svg/pdf; masks and boolean operands with stacks |
| P3 | Panel rows (here: in the Appearance panel, one list with VectorCraft's effects) | — | Not yet ported. Rule for the one list: see [`modifiers.md`](modifiers.md). picabia-craft gap: linear-array icon reads slightly small |
| P4 | Circular, grid, mirror, transform each, randomize; stacking and caps | — | Not yet ported (the kinds evaluate in the P1 crate). picabia-craft gap: no 10k-copy interactivity benchmark |
| P5 | `modifier.apply`, menus, canvas handles, layers badge | — | Not yet ported. picabia-craft gaps: no handles for wallpaper, transform each, randomize, copy to shape, or for the row whose settings are open |
| P6 | Parameters (replaces picabia-craft's variable bindings) | — | Not ported as such: no `bind.rs`. Replaced by the parameters design (parameters in the Variables panel, a parameter/expression/ramp chip on every number field). Per-copy expressions stay in the stack format |
| P7 | Wallpaper groups and copy to shape | — | Not yet ported (both kinds evaluate in the P1 crate). picabia-craft gap: no canvas pick-a-layer tool for the target |
| P8–P11 | Stencil: path ops, sheet, islands and ties, repeat, limits and materials | — | Not yet ported (the `stencil` kind parses in the P1 crate). picabia-craft gaps: ties layer and text option not built; in fix mode narrow checks only look near ties, registration holes and edges; pattern ties ignore the repeat; material presets are guesses until measured with the test card |
| P12 | Stencil laser export and test card | C | Engine ported (`modifier.stencilExport`, `modifier.stencilTestCard`, `modifier.stencilGcode`, `laser.gcode`). UI (PIC9.6, PIC15.20, PIC17.12): the Stencil window's settings (Colours, Size, Clean-up, Ties, Repeat, Registration, Preview), stats, problem rows and canvas markers; the Export for Laser / Laser Test Card / Export G-code for Laser dialog with Format (SVG for LightBurn, G-code for GRBL), material and operation table, summary from a dry run. picabia-craft gaps: never opened in laser software or cut; score lines cross cut-outs; contour direction not normalised; no DXF; tie bars and the repeat cell aren't drawn on the canvas here (problems and registration holes are) |
| P13, P14 | Image Trace and Pathfinder generator rows | n/a | Not ported: VectorCraft's own Image Trace and live Pathfinder are used. The trace to stencil path must still work |
| P16 | Interface scope (hide prototyping etc.) | n/a | Not ported: Picabia on VectorCraft hides nothing. PIC16.1 (no VectorCraft update check) is ported, see P0 |
| P17 | Laser control: G-code and a GRBL sender | C | Link and `laser.*` ported (simulator-tested). Laser window (PIC17.11, PIC17.12): connect, state, Home/Unlock, jog pad, Set origin, Frame, the job (selection, test card, power ladder, G-code file; sheet, material, operation table, summary), Start through the safety checklist (the only arming; injected control-channel clicks arm only the simulator), Pause/Resume (confirm), the always-visible Stop and Shift+Esc, progress, alarms, the machine profile editor with per-material settings; real ports on in the desktop app (`PICABIA_LASER_HARDWARE=0` keeps them off). Safety review fixes (PIC17.11a): accessibility actions and injected input never arm or move a real machine, motion commands on a real machine are the window's person's only, real ports off under `--control` unless `PICABIA_LASER_HARDWARE=1`, Shift+Esc from floating panels' windows, no file dialogs while the laser works. Gaps: never connected to a real GRBL or cut; while a menu-bar menu is open no frames run, so Stop and Shift+Esc wait (the machine's emergency stop must be in reach); a keyboard press of Start counts as automation; no canvas overlay of the head and job; browser build has no laser control |

## P0 identity, as built

- **Name and id.** `crates/ui-egui/src/picabia.rs`: `APP_NAME = "Picabia"`, `APP_ID =
  "org.picabia.Picabia"` (Wayland `app_id` / X11 class), the repo and upstream links and the About
  credits. Window title ("file — Picabia"), app bar title, the macOS app menu's name, the About
  window's title and name, the viewport title and `run_native` name use them.
- **Settings folder.** `apps/vectorcraft/src/picabia.rs` (loaded by `prefs_dir.rs`, so
  `vectorcraft-cli` shares it): `~/Library/Application Support/Picabia` on macOS, `%APPDATA%\Picabia`
  on Windows, `$XDG_CONFIG_HOME/picabia` (or `~/.config/picabia`) on Linux and BSD; a portable copy
  keeps `PicabiaData` beside itself. Everything the app keeps lives there (`ui.json`, logs, Data
  Recovery, Fonts, Swatches, Graphic Styles, Symbols, Brushes, Libraries, Color Profiles, Plug-ins,
  PDF Presets, the single-instance lock), so Picabia and VectorCraft never read or overwrite each
  other's. The one-time read of DrawCraft's settings (VectorCraft's former name) is off for Picabia;
  VectorCraft's migration code is untouched. `PICABIA_CONFIG_DIR` and `PICABIA_NO_PREFS` override
  it and win over `VECTORCRAFT_CONFIG_DIR` / `VECTORCRAFT_NO_PREFS`, which still apply when no
  Picabia variable is set (agents' isolated runs use them).
- **About.** Picabia's icon and name, the version, what Picabia is, links to Picabia's repository and
  to VectorCraft, and the credit "Includes VectorCraft by the ArtCraft team and community (MIT OR
  Apache-2.0)" with the font and icon credits. No ArtCraft logo, Discord or website links.
- **Icon.** `assets/picabia/app-icon/`: an original placeholder (eight dots on a ring), the runtime
  window/Dock icon and the in-app brand mark (app bar, About). VectorCraft's own icon files stay.
- **Kept on purpose.** Crate and package names, the `vectorcraft` executable, `.vectorcraft` files,
  the `VECTORCRAFT_*` environment variables and the `--control` scripting stay VectorCraft's:
  renaming them would conflict on every upstream merge.

## Gaps

- **Packaging identity.** The macOS bundle (`Info.plist` name and `CFBundleIdentifier`,
  `vectorcraft.icns`), the Windows resources and installer (`vectorcraft.ico`, product name), the
  Linux `.desktop`, metainfo, mime and `hicolor/` icons (`ai.storyteller.vectorcraft.*`), Flatpak
  and the web build's title and favicon are still VectorCraft's. The Linux `.desktop` file's
  `StartupWMClass` no longer matches the window's app id, so a Wayland dock won't match a launched
  window to an installed launcher until Picabia ships its own desktop entry.
- **Packaged window class.** Packaging still uses VectorCraft's ids: `packaging/linux`
  (`ai.storyteller.vectorcraft.desktop` with `StartupWMClass=ai.storyteller.vectorcraft`, metainfo,
  mime, `hicolor/`), the FreeBSD and Flatpak manifests, the desktop-file test in
  `apps/vectorcraft/src/tests.rs`, and `xtask/src/bundle.rs` (macOS bundle id and `CFBundleName`).
  The window's app id is now `org.picabia.Picabia`, so a packaged install's launcher won't match
  the running window until packaging is rebranded.
- **Texts still naming VectorCraft.** Done (PIC0.6a): the macOS app menu (About, Hide, Quit), Help ▸
  About, Help ▸ GitHub (now "Picabia on GitHub", opening github.com/profdl/picabia-vector), the
  Recover Documents text. Left: "VectorCraft Help…" (not built), Help ▸ "VectorCraft on
  getartcraft.com" (kept as the upstream link), the "ArtCraft Website" and "Join Our Discord" Help
  items, the "VectorCraft Updates" web window title, messages such as "restart VectorCraft", the
  file-type name "VectorCraft" for `.vectorcraft` files, and log lines. The Picabia About texts are
  English only.
- **Discord.** Hidden from the app bar and the home screen (PIC0.6a); Help ▸ Join Our Discord and the
  macOS app menu's Discord item remain (they are upstream's community).
- **Updates (PIC16.1).** No check at startup; Help ▸ Check for Updates opens
  github.com/profdl/picabia-vector/releases instead of VectorCraft's checker, whose feed would
  offer VectorCraft. Picabia's own feed comes with its release packaging.
- **Binary name.** Still `vectorcraft`; a `[[bin]] name = "picabia"` entry is the least invasive
  rename, at the first release.

## Licence notes

The owner authored tlprocedural and gave permission to port it; that note goes into `NOTICE` with
the first ported tlprocedural feature (PIC20 on). The Picabia icon is original work (see
`assets/picabia/app-icon/`), attributed in `ASSETS.md` and `NOTICE`.

## Links

- Modifiers spec: [`modifiers.md`](modifiers.md)
- Seams register: [`SEAMS.md`](SEAMS.md)
- VectorCraft's own docs: [`../../ROADMAP.md`](../../ROADMAP.md), [`../gaps.md`](../gaps.md)
