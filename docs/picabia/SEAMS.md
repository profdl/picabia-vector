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
| `crates/ui-egui/src/menus/labels.rs` | `display_label` | Items that name the app (`help.about`, `app.quit`, `help.github`) say Picabia, translated first | Re-add the first match arm `_ if crate::picabia::names_the_app(id) => …rename(tr_id(…))` |
| `crates/ui-egui/src/native_menu.rs` | `mac_layout` hide and quit items | macOS app menu "Hide Picabia", "Quit Picabia" | Wrap the two labels in `crate::picabia::rename` |
| `crates/ui-egui/src/dialogs/recovery.rs` | `SPEC.body` | "Picabia didn't quit normally…" | Wrap the `tl!` text in `crate::picabia::rename` |
| `crates/engine/src/cmd/help.rs` | `github_url`, its test | Help ▸ GitHub opens github.com/profdl/picabia-vector; the getartcraft.com page stays VectorCraft's (the upstream link) | Return Picabia's URL; the test expects it |
| `crates/ui-egui/src/community.rs` | `link_clicks_open_the_right_urls` test | Expects Picabia's GitHub URL | Change the last expected URL |
| `crates/ui-egui/src/chrome.rs` | `app_bar` `discord` | No VectorCraft Discord button in the app bar | Prefix the condition with `crate::picabia::SHOWS_DISCORD &&` |
| `crates/ui-egui/src/canvas/home.rs` | Community links | Home screen shows Picabia's and VectorCraft's links, not ArtCraft's Discord and website | Call `crate::picabia::links(ui)` instead of `community::links` |
| `crates/ui-egui/src/menus/run.rs` | `help.checkForUpdates` (PIC16.1) | VectorCraft's checker would offer VectorCraft's releases: open Picabia's releases page instead | Re-add the guarded arm `if !crate::picabia::HAS_UPDATE_FEED` above upstream's |
| `apps/vectorcraft/src/main.rs` | `set_startup_update_check` (PIC16.1) | No update check at startup | Pass `false` |
| `apps/vectorcraft/src/main.rs`, `prefs.rs`, `prefs_dir.rs` (`store_dir`), `profile.rs` (`prefs_path_with` + a test) | `VECTORCRAFT_NO_PREFS` / `VECTORCRAFT_CONFIG_DIR` reads | `PICABIA_NO_PREFS` / `PICABIA_CONFIG_DIR` read first, so a global VectorCraft override needn't make Picabia share a folder | Read through `picabia::var_os` / `picabia::env_first`; `prefs_dir`'s `mod picabia` is `pub(crate)` |
| `crates/ui-egui/src/tests/nativemenu.rs`, `crates/ui-egui/src/titlebar.rs` (test) | `vectorcraft_gets_a_mac_app_menu`, `caption_buttons_take_the_right_end…` | Expect "About/Quit/Hide Picabia" and no Discord button | Re-apply the expected strings and the `SHOWS_DISCORD &&` |
