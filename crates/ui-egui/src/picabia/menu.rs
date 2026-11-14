//! PICABIA: Object ▸ Modifiers and Object ▸ Physics, in the menu tree (`menus::tree`), which also builds the macOS menu
//! bar and the command palette.

use serde_json::json;

use crate::menus::Item;

/// Picabia's View menu commands that the app runs itself and that can be steps of an action
/// (`menus::plays_in_app`, beside upstream's list): Show/Hide Modifier Copy Outlines.
pub const UI_STEP_COMMANDS: [&str; 1] = ["view.modifierCopyOutlines"];

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

/// The Object ▸ Physics submenu: Simulate to Rest and Start Simulation (a live session with the
/// transport bar), Drop and Apply (phase 1's instant drop, one undo step), settings for the
/// selection (dynamic, static, soft presets, or open them in the Appearance panel), and Clear.
pub fn physics_items() -> Vec<Item> {
    vec![
        // Simulate to Rest runs the session to rest and shows it on the canvas with the transport
        // bar (Apply or Cancel); Start Simulation plays it live.
        Item::Cmd("Simulate to Rest", "physics.toRest", json!({})),
        Item::Cmd("Start Simulation", "physics.play", json!({})),
        // `{}`, not null: the app adds the active artboard (the walls) to an object's params.
        Item::Cmd("Drop and Apply", "physics.simulate", json!({})),
        Item::Sep,
        Item::Cmd("Add Physics", "physics.set", json!({"props": {"enabled": true}})),
        Item::Cmd("Make Static", "physics.set", json!({"props": {"bodyType": "static"}})),
        Item::Cmd("Make Cloth", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "cloth"}}})),
        Item::Cmd("Make Jelly", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "jelly"}}})),
        Item::Cmd("Make Rubber", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "rubber"}}})),
        Item::Cmd("Settings…", crate::picabia_ui::PHYSICS_SETTINGS, json!({})),
        Item::Sep,
        // Pins: anchors of a soft object chosen with Direct Selection.
        Item::Cmd("Pin Points", "physics.pin", json!({})),
        Item::Cmd("Attach Points to Selected", "physics.pin", json!({"attachToSelected": true})),
        Item::Cmd("Unpin Points", "physics.unpin", json!({})),
        Item::Sep,
        Item::Cmd("Clear Physics", "physics.clear", serde_json::Value::Null),
    ]
}
