//! Picabia's identity as the window shows it: the app name, the desktop app id and the About
//! window's credits. Upstream files reach these through short `// PICABIA:` seams
//! (`docs/picabia/SEAMS.md`). The settings folder is set beside the settings path, in
//! `apps/vectorcraft/src/picabia.rs` (shared with `vectorcraft-cli`, which has no UI).
//!
//! Crate, package, binary and file-format names stay VectorCraft's (`.vectorcraft` files, the
//! `vectorcraft` executable): renaming them would conflict on every upstream merge.

pub mod add_menu;
pub mod appearance;
#[cfg(test)]
mod appearance_tests;
pub mod badge;
pub mod bind_chip;
#[cfg(test)]
mod bind_chip_tests;
#[cfg(test)]
mod discover_tests;
pub mod menu;
pub mod params_panel;
pub mod physics;
#[cfg(test)]
mod physics_tests;
pub mod properties;
pub mod sim_bar;
#[cfg(test)]
mod sim_bar_tests;
pub mod sim_poke;
#[cfg(test)]
mod sim_poke_tests;
pub mod task_bar;

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// The name the app shows: window title, app bar, macOS app menu, About window.
pub const APP_NAME: &str = "Picabia";
/// Wayland `app_id` / X11 window class (the desktop entry's name, once Picabia ships one).
pub const APP_ID: &str = "org.picabia.Picabia";
/// Picabia's source code.
pub const REPO_URL: &str = "https://github.com/profdl/picabia-vector";
/// The app Picabia is built on.
pub const UPSTREAM_NAME: &str = "VectorCraft";
/// VectorCraft's source code.
pub const UPSTREAM_URL: &str = "https://github.com/storytold/vectorcraft";
/// Where Picabia's releases are published (Help ▸ Check for Updates opens it).
pub const RELEASES_URL: &str = "https://github.com/profdl/picabia-vector/releases";
/// Whether Picabia has a release feed of its own for VectorCraft's update checker (not yet).
pub const HAS_UPDATE_FEED: bool = false;
/// VectorCraft's Discord button (its community, not Picabia's) is left out of the app bar.
pub const SHOWS_DISCORD: bool = false;

/// `text` with VectorCraft's name replaced by Picabia's, for an upstream label that names the app
/// ("About VectorCraft"), translated first so languages that keep the Latin name read right.
pub fn rename(text: &str) -> String {
    text.replace(UPSTREAM_NAME, APP_NAME)
}

/// Menu items whose label names the app (About, Quit, the GitHub link), by command id: their
/// translated label gets Picabia's name.
pub fn names_the_app(id: &str) -> bool {
    matches!(id, "help.about" | "app.quit" | "help.github")
}

/// Help ▸ Check for Updates. VectorCraft's checker reads VectorCraft's release feed, whose download
/// would replace Picabia with VectorCraft, and Picabia has no feed of its own yet: open Picabia's
/// releases page instead. (Picabia follows VectorCraft by merging upstream.)
pub fn check_for_updates(app: &mut crate::VectorcraftApp) -> Result<serde_json::Value, String> {
    app.open_url(RELEASES_URL);
    Ok(serde_json::json!({ "url": RELEASES_URL }))
}

/// Picabia's two links (its repository and VectorCraft's), for the About window and the home screen.
pub fn links(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.hyperlink_to(RichText::new("github.com/profdl/picabia-vector").color(t.accent), REPO_URL);
    ui.hyperlink_to(RichText::new(format!("{UPSTREAM_NAME}, the open-source app {APP_NAME} is built on")).color(t.accent), UPSTREAM_URL);
}

/// The About tab's middle part: what Picabia is, its links and the upstream credit (in place of
/// VectorCraft's community links and "Part of ArtCraft" line). The ArtCraft name and logos are
/// upstream's trademarks, so Picabia neither shows nor links them as its own.
pub fn about_credits(ui: &mut Ui) {
    ui.label(format!(
        "{APP_NAME} is built on {UPSTREAM_NAME}: vector illustration with a live modifier stack (arrays, mirrors and \
         randomised repeats that stay editable)."
    ));
    ui.add_space(8.0);
    links(ui);
    ui.add_space(12.0);
    ui.label(
        RichText::new(format!(
            "MIT OR Apache-2.0. Includes {UPSTREAM_NAME} by the ArtCraft team and community (MIT OR Apache-2.0). \
             Fonts: Source Sans 3, Inter, JetBrains Mono (OFL). Icons: Lucide (ISC) + {UPSTREAM_NAME}. \
             App icon: original, by the {APP_NAME} contributors."
        ))
        .size(11.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_labels_name_picabia() {
        assert_eq!(rename("About VectorCraft"), "About Picabia");
        assert_eq!(rename("À propos de VectorCraft"), "À propos de Picabia");
        assert_eq!(rename("Swatches"), "Swatches");
    }

    #[test]
    fn check_for_updates_opens_picabias_releases_not_vectorcrafts_feed() {
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let r = app.run("help.checkForUpdates", serde_json::json!({}));
        assert_eq!(r, Ok(serde_json::json!({ "url": RELEASES_URL })));
        assert_eq!(app.pending_url.as_deref(), Some(RELEASES_URL));
        // (Upstream's `updates_ui::check` is not reached: the result above is Picabia's, which queues no check.)
    }

    #[test]
    fn menu_items_that_name_the_app_say_picabia() {
        let app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        for (id, label, want) in [
            ("help.about", "About VectorCraft", "About Picabia"),
            ("app.quit", "Quit VectorCraft", "Quit Picabia"),
            ("help.github", "VectorCraft on GitHub", "Picabia on GitHub"),
        ] {
            assert_eq!(crate::menus::display_label(&app, id, label), want);
        }
        // Upstream's own page keeps its name: it is a link to VectorCraft.
        assert_eq!(crate::menus::display_label(&app, "help.appPage", "VectorCraft on getartcraft.com"), "VectorCraft on getartcraft.com");
    }

    #[test]
    fn the_about_credits_draw_headlessly() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), about_credits);
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
    }
}
