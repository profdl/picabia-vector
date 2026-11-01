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
