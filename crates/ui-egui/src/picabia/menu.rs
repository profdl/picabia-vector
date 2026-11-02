//! PICABIA: Object ▸ Modifiers and Object ▸ Physics, in the menu tree (`menus::tree`), which also builds the macOS menu
//! bar and the command palette.

use serde_json::json;

use crate::menus::Item;

/// The Object ▸ Modifiers submenu: add each kind to the selection (and the screen's presets), in
/// the groups of the panels' Add Modifier menu (`super::add_menu::GROUPS`), then Apply, Remove All,
/// and copying and pasting rows between objects.
pub fn object_items() -> Vec<Item> {
    let mut items = vec![];
    // The menu bar's submenus have no headings: separators part the groups.
    for (_, entries) in super::add_menu::GROUPS {
        if !items.is_empty() {
            items.push(Item::Sep);
        }
        items.extend(entries.iter().map(|e| Item::Cmd(e.label, "modifier.add", super::add_menu::params(e))));
    }
    items.extend([
        Item::Sep,
        Item::Cmd("Apply Modifiers", "modifier.apply", serde_json::Value::Null),
        Item::Cmd("Remove All Modifiers", "modifier.remove_all", serde_json::Value::Null),
        Item::Sep,
        Item::Cmd("Copy Modifiers", "modifier.copy", serde_json::Value::Null),
        Item::Cmd("Paste Modifiers", "modifier.paste", serde_json::Value::Null),
    ]);
    items
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
