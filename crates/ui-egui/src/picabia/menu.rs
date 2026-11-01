//! PICABIA: Object ▸ Modifiers and Object ▸ Physics, in the menu tree (`menus::tree`), which also builds the macOS menu
//! bar and the command palette.

use serde_json::json;

use crate::menus::Item;

/// The Object ▸ Modifiers submenu: add each kind to the selection, then Apply, Remove All, and
/// copying and pasting rows between objects.
pub fn object_items() -> Vec<Item> {
    let add = |label: &'static str, kind: &str| Item::Cmd(label, "modifier.add", json!({ "kind": kind }));
    vec![
        add("Linear Array", "linearArray"),
        add("Circular Array", "circularArray"),
        add("Grid Array", "gridArray"),
        add("Mirror", "mirror"),
        add("Wallpaper", "wallpaper"),
        add("Copy to Shape", "copyToShape"),
        add("Transform Each", "transformEach"),
        add("Randomize", "randomize"),
        add("Stencil", "stencil"),
        Item::Sep,
        add("Resample", "resample"),
        add("Trim", "trim"),
        add("Tidy", "tidy"),
        Item::Sep,
        Item::Cmd("Apply Modifiers", "modifier.apply", serde_json::Value::Null),
        Item::Cmd("Remove All Modifiers", "modifier.remove_all", serde_json::Value::Null),
        Item::Sep,
        Item::Cmd("Copy Modifiers", "modifier.copy", serde_json::Value::Null),
        Item::Cmd("Paste Modifiers", "modifier.paste", serde_json::Value::Null),
    ]
}

/// The Object ▸ Physics submenu: Simulate (drop and settle, one undo step), settings for the
/// selection (dynamic, static, or open them in the Appearance panel), and Clear.
pub fn physics_items() -> Vec<Item> {
    vec![
        // `{}`, not null: the app adds the active artboard (the walls) to an object's params.
        Item::Cmd("Simulate", "physics.simulate", json!({})),
        Item::Sep,
        Item::Cmd("Add Physics", "physics.set", json!({"props": {"enabled": true}})),
        Item::Cmd("Make Static", "physics.set", json!({"props": {"bodyType": "static"}})),
        Item::Cmd("Settings…", crate::picabia_ui::PHYSICS_SETTINGS, json!({})),
        Item::Sep,
        Item::Cmd("Clear Physics", "physics.clear", serde_json::Value::Null),
    ]
}
