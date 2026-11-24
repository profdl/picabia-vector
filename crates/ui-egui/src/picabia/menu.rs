//! PICABIA: Object ▸ Modifiers and Object ▸ Physics, in the menu tree (`menus::tree`), which also builds the macOS menu
//! bar and the command palette.

use serde_json::json;

use crate::menus::{Node, Sep, cp};

/// Picabia's View menu commands that the app runs itself and that can be steps of an action
/// (`menus::plays_in_app`, beside upstream's list): Show/Hide Modifier Copy Outlines.
pub const UI_STEP_COMMANDS: [&str; 1] = ["view.modifierCopyOutlines"];

/// The Object ▸ Modifiers submenu: add each kind to the selection (and the screen's presets), in
/// the groups of the panels' Add Modifier menu (`super::add_menu::GROUPS`), then Apply, Remove All,
/// and copying and pasting rows between objects.
pub fn object_items() -> Vec<Node> {
    let mut items = vec![];
    // The menu bar's submenus have no headings: separators part the groups.
    for (_, entries) in super::add_menu::GROUPS {
        if !items.is_empty() {
            items.push(Sep);
        }
        items.extend(entries.iter().map(|e| cp(e.label, "modifier.add", super::add_menu::params(e))));
    }
    items.extend([
        Sep,
        cp("Apply Modifiers", "modifier.apply", serde_json::Value::Null),
        cp("Remove All Modifiers", "modifier.remove_all", serde_json::Value::Null),
        Sep,
        cp("Copy Modifiers", "modifier.copy", serde_json::Value::Null),
        cp("Paste Modifiers", "modifier.paste", serde_json::Value::Null),
    ]);
    items
}

/// The Object ▸ Physics submenu: Simulate to Rest and Start Simulation (a live session with the
/// transport bar), Drop and Apply (phase 1's instant drop, one undo step), settings for the
/// selection (dynamic, static, soft presets, or open them in the Appearance panel), and Clear.
pub fn physics_items() -> Vec<Node> {
    vec![
        // Simulate to Rest runs the session to rest and shows it on the canvas with the transport
        // bar (Apply or Cancel); Start Simulation plays it live.
        cp("Simulate to Rest", "physics.toRest", json!({})),
        cp("Start Simulation", "physics.play", json!({})),
        // `{}`, not null: the app adds the active artboard (the walls) to an object's params.
        cp("Drop and Apply", "physics.simulate", json!({})),
        Sep,
        cp("Add Physics", "physics.set", json!({"props": {"enabled": true}})),
        cp("Make Static", "physics.set", json!({"props": {"bodyType": "static"}})),
        cp("Make Cloth", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "cloth"}}})),
        cp("Make Jelly", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "jelly"}}})),
        cp("Make Rubber", "physics.set", json!({"props": {"bodyType": "soft", "soft": {"preset": "rubber"}}})),
        cp("Settings…", crate::picabia_ui::PHYSICS_SETTINGS, json!({})),
        Sep,
        // Pins: anchors of a soft object chosen with Direct Selection.
        cp("Pin Points", "physics.pin", json!({})),
        cp("Attach Points to Selected", "physics.pin", json!({"attachToSelected": true})),
        cp("Unpin Points", "physics.unpin", json!({})),
        Sep,
        cp("Clear Physics", "physics.clear", serde_json::Value::Null),
    ]
}

/// The command rows of `nodes` (not their submenus' rows): label, command id and params.
#[cfg(test)]
pub(crate) fn commands(nodes: &[Node]) -> Vec<(String, String, serde_json::Value)> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Item(it) => Some((it.label.clone(), it.command.id.to_string(), it.command.params.clone())),
            _ => None,
        })
        .collect()
}

/// The rows of the submenu of `nodes` titled `label`.
#[cfg(test)]
pub(crate) fn submenu<'a>(nodes: &'a [Node], label: &str) -> Option<&'a [Node]> {
    nodes.iter().find_map(|n| match n {
        Node::Submenu { label: l, children, .. } if l == label => Some(children.as_slice()),
        _ => None,
    })
}
