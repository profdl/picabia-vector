//! Picabia's identity as the window shows it: the app name, the desktop app id and the About
//! window's credits. Upstream files reach these through short `// PICABIA:` seams
//! (`docs/picabia/SEAMS.md`). The settings folder is set beside the settings path, in
//! `apps/vectorcraft/src/picabia.rs` (shared with `vectorcraft-cli`, which has no UI).
//!
//! Crate, package, binary and file-format names stay VectorCraft's (`.vectorcraft` files, the
//! `vectorcraft` executable): renaming them would conflict on every upstream merge.

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

/// `text` with VectorCraft's name replaced by Picabia's, for an upstream label that names the app
/// ("About VectorCraft"), translated first so languages that keep the Latin name read right.
pub fn rename(text: &str) -> String {
    text.replace(UPSTREAM_NAME, APP_NAME)
}

/// The About tab's middle part: what Picabia is, its links and the upstream credit (in place of
/// VectorCraft's community links and "Part of ArtCraft" line). The ArtCraft name and logos are
/// upstream's trademarks, so Picabia neither shows nor links them as its own.
pub fn about_credits(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(format!(
        "{APP_NAME} is built on {UPSTREAM_NAME}: vector illustration with a live modifier stack (arrays, mirrors and \
         randomised repeats that stay editable)."
    ));
    ui.add_space(8.0);
    ui.hyperlink_to(RichText::new("github.com/profdl/picabia-vector").color(t.accent), REPO_URL);
    ui.hyperlink_to(RichText::new(format!("{UPSTREAM_NAME}, the open-source app {APP_NAME} is built on")).color(t.accent), UPSTREAM_URL);
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
    fn the_about_credits_draw_headlessly() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), about_credits);
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
    }
}
