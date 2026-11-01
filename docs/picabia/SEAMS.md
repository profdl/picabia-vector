# Picabia seams in upstream VectorCraft files

Every edit Picabia makes to a file that comes from upstream VectorCraft is a *seam*: as few lines as
possible, calling into Picabia code, marked `// PICABIA: <reason>` (`rg "PICABIA:"` lists them).
When an upstream merge conflicts, resolve it seam by seam with this table, then run `cargo xtask ci`.

| File | Function / item | Why | How to re-apply |
|---|---|---|---|
| `.github/workflows/release.yml` (`release` job), `sonarcloud.yml`, `freebsd.yml`, `windows-arm64.yml` | top job | Need storytold signing secrets, SonarCloud token or its runners/schedules | Re-add `if: github.repository == 'storytold/vectorcraft'` under the job name (4 lines in all). `ci.yml`, `nix.yml`, `packaging-lint.yml`, `affinity-corpus.yml` and `print.yml` run unchanged on GitHub-hosted runners. |
| `apps/vectorcraft/src/titlebar.rs` | `app_icon`, `native_options` | Runtime window/Dock icon is Picabia's (`assets/picabia/app-icon/`); viewport title and app id are Picabia's | Point the two `include_bytes!` at `picabia-macos-512.png` / `picabia-256.png`; `.with_title(vectorcraft_ui_egui::picabia::APP_NAME)`, `.with_app_id(vectorcraft_ui_egui::picabia::APP_ID)` |
| `crates/ui-egui/src/brand.rs` | `ICON_PNG` | In-app brand mark (app bar, About) is Picabia's icon | Point the `include_bytes!` at `assets/picabia/app-icon/picabia-128.png` |
| `apps/vectorcraft/src/main.rs` | `run` closure | `run_native` app name | Pass `vectorcraft_ui_egui::picabia::APP_NAME` in both arms |
| `apps/vectorcraft/src/prefs_dir.rs` + new `apps/vectorcraft/src/picabia.rs` | module list | Loads Picabia's settings-folder constants beside `profile.rs` (shared with `vectorcraft-cli`) | Re-add `#[path = "picabia.rs"] mod picabia;` after `mod profile;` |
| `apps/vectorcraft/src/profile.rs` | `PORTABLE_DATA_DIR`, `prefs_path`, `platform_dir`, 4 test asserts | Settings live in Picabia's own folder (`Picabia` / `picabia`, portable `PicabiaData`), and DrawCraft's old settings are never read, so Picabia and VectorCraft never share preferences | Re-point the constant and the non-legacy names at `super::picabia::*`; keep the `legacy && !READS_DRAWCRAFT_PREFS` early return in `prefs_path`; the tests expect `Picabia`/`picabia` |
| `crates/ui-egui/src/lib.rs` | module list | `pub mod picabia;` (name, id, About credits) | Re-add the line after `pub mod panels;` |
| `crates/ui-egui/src/native_menu.rs` | `APP_NAME` | macOS app menu title "Picabia" | `= crate::picabia::APP_NAME` |
| `crates/ui-egui/src/chrome.rs`, `chrome/tests.rs` | `app_bar` title, `window_title`, app-bar overlap test, title tests | Window and app-bar title "Picabia" ("file — Picabia") | Use `crate::picabia::APP_NAME` in place of the `"VectorCraft"` literals; tests expect "Picabia" |
| `crates/ui-egui/src/dialogs/about.rs` | `show` title, `about_tab` | About: "About Picabia" (translated title with the name swapped), Picabia's name, `picabia::about_credits` (Picabia and VectorCraft links, upstream credit, licences) in place of the ArtCraft community links and "Part of ArtCraft" line | Re-apply the 3 seams; the Contributors and Models tabs and the GPU line are upstream's |
| `README.md` | top of file | Picabia header (icon, name, links, fork note) replaces the ArtCraft logo block; VectorCraft's README follows | Keep our header, upstream text below the rule |
| `NOTICE` | top of file | Picabia's copyright, fork and icon notice above VectorCraft's, which follows unchanged | Keep our block on top |
| `ASSETS.md` | rows after `assets/app-icon/` | Rows for `assets/picabia/app-icon/*` | Keep the 6 rows |
| `crates/ui-egui/src/tests/nativemenu.rs`, `crates/ui-egui/src/i18n/tests.rs` | `vectorcraft_gets_a_mac_app_menu`, `language_command_sets_the_preference` | The macOS app menu is titled "Picabia" | Expect `"Picabia"` / `crate::picabia::APP_NAME` as the app menu's title |
