//! PICABIA: Object ▸ Modifiers, in the menu tree (`menus::tree`), which also builds the macOS menu
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
        add("Deform", "deform"),
        add("Displace", "displace"),
        Item::Sep,
        Item::Cmd("Apply Modifiers", "modifier.apply", serde_json::Value::Null),
        Item::Cmd("Remove All Modifiers", "modifier.remove_all", serde_json::Value::Null),
        Item::Sep,
        Item::Cmd("Copy Modifiers", "modifier.copy", serde_json::Value::Null),
        Item::Cmd("Paste Modifiers", "modifier.paste", serde_json::Value::Null),
    ]
}
