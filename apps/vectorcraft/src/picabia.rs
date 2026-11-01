//! Picabia's settings folder, kept apart from VectorCraft's so the two apps run side by side
//! without reading or overwriting each other's preferences (and, later, Picabia's own laser and
//! machine settings). Loaded beside `profile.rs` by `prefs_dir.rs`, so `vectorcraft-cli` (which
//! shares that file) reads the same folder. The app name and id the window shows live in
//! `vectorcraft_ui_egui::picabia`.
//!
//! Everything the app keeps sits in this folder: `ui.json`, logs, Data Recovery, Fonts, Swatches,
//! Graphic Styles, Symbols, Brushes, Libraries, Color Profiles, Plug-ins, PDF Presets and the
//! single-instance lock.

/// The settings folder's name in ~/Library/Application Support (macOS) and %APPDATA% (Windows).
pub const CONFIG_DIR: &str = "Picabia";
/// The settings folder's name in $XDG_CONFIG_HOME or ~/.config (Linux and BSD).
pub const CONFIG_DIR_LOWER: &str = "picabia";
/// The settings folder a portable copy creates beside itself (VectorCraft's is `VectorCraftData`).
pub const PORTABLE_DATA_DIR: &str = "PicabiaData";
/// Whether a first start without Picabia settings reads DrawCraft's (VectorCraft's former name)
/// once. Off: those are another app's settings, and Picabia never had a DrawCraft profile.
/// VectorCraft's migration code stays as it is, so upstream merges and its tests are untouched.
pub const READS_DRAWCRAFT_PREFS: bool = false;

/// The environment variable `key` as the settings code reads it, with Picabia's own name for it
/// first: `PICABIA_CONFIG_DIR` before `VECTORCRAFT_CONFIG_DIR`, `PICABIA_NO_PREFS` before
/// `VECTORCRAFT_NO_PREFS`. A user who points every VectorCraft at one folder can give Picabia its
/// own; without a Picabia variable the VectorCraft one still applies (agents' and tests' isolated
/// runs set it).
pub fn env_first(env: &impl Fn(&str) -> Option<std::ffi::OsString>, key: &str) -> Option<std::ffi::OsString> {
    let own = match key {
        "VECTORCRAFT_CONFIG_DIR" => Some("PICABIA_CONFIG_DIR"),
        "VECTORCRAFT_NO_PREFS" => Some("PICABIA_NO_PREFS"),
        _ => None,
    };
    own.and_then(env).or_else(|| env(key))
}

/// [`env_first`] on the process environment.
pub fn var_os(key: &str) -> Option<std::ffi::OsString> {
    env_first(&|k: &str| std::env::var_os(k), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).into())
    }

    #[test]
    fn picabias_own_variables_win_over_vectorcrafts() {
        let both = env(&[("PICABIA_CONFIG_DIR", "/p"), ("VECTORCRAFT_CONFIG_DIR", "/v"), ("PICABIA_NO_PREFS", "1")]);
        assert_eq!(env_first(&both, "VECTORCRAFT_CONFIG_DIR"), Some("/p".into()));
        assert_eq!(env_first(&both, "VECTORCRAFT_NO_PREFS"), Some("1".into()));
        let upstream_only = env(&[("VECTORCRAFT_CONFIG_DIR", "/v")]);
        assert_eq!(env_first(&upstream_only, "VECTORCRAFT_CONFIG_DIR"), Some("/v".into()));
        assert_eq!(env_first(&upstream_only, "VECTORCRAFT_NO_PREFS"), None);
        assert_eq!(env_first(&both, "HOME"), None, "other variables are read as they are");
    }
}
