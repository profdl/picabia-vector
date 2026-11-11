//! PICABIA (PIC3.4): one command per modifier kind and screen or color preset
//! (`modifier.add.linearArray`, `modifier.add.benDay`, `modifier.add.rainbow`, …), each `modifier.add` with its kind (and preset) filled in.
//!
//! They exist so the command palette (which runs a command with no params) finds each kind by
//! name ("Linear Array", "Ben-Day Dots") and so a keyboard shortcut can be given to one in the
//! shortcut editor (Object › Modifiers). `modifier.add` stays the command agents and the panels
//! use; these only fill in `kind` and `preset`.

use serde_json::{Map, Value, json};

use super::add_with;
use crate::cmd::{CommandSpec, bad, has_selection};
use crate::{Result, Session};

/// What each of them takes and returns.
const PARAMS: &str = "{ids?|id?: the objects (default: the selection), props?: the kind's params as modifier.add takes them, index?: where in the stack (default: the end), kind?, preset?: refused unless the command's own} modifier.add with this command's kind (and screen preset), as one undo step → {modifier, added}";

/// The menu path they are listed under (the palette's and the shortcut editor's group).
const MENU: &[&str] = &["Object", "Modifiers"];

/// `modifier.add` with `kind` (and `preset`) over `p`'s other params, as command `cmd`, recorded
/// as undo step `label` ("Add Linear Array"). A `kind` or `preset` in `p` other than the command's
/// own is refused rather than overwritten.
fn add_as(s: &mut Session, p: &Value, cmd: &str, label: &str, kind: &str, preset: Option<&str>) -> Result<Value> {
    let mut q = match p {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    for (key, want) in [("kind", Some(kind)), ("preset", preset)] {
        match q.get(key) {
            None | Some(Value::Null) => {}
            Some(v) if v.as_str().is_some() && v.as_str() == want => {}
            Some(v) => return Err(bad(cmd, format!("`{key}` is this command's own ({}); not {v}", want.unwrap_or("none")))),
        }
    }
    q.insert("kind".into(), json!(kind));
    if let Some(preset) = preset {
        q.insert("preset".into(), json!(preset));
    }
    add_with(s, &Value::Object(q), cmd, label)
}

/// One spec: `id`, `label` and what it runs.
fn spec(id: &'static str, label: &'static str, run: crate::cmd::Run) -> CommandSpec {
    CommandSpec { id, label, menu: MENU, shortcut: None, params: PARAMS, enabled: has_selection, run, journal: true, schema: None }
}

/// The commands, one per kind (`modifier.add.<kind>`) and screen preset (`modifier.add.<preset>`).
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        spec("modifier.add.linearArray", "Linear Array", |s, p| add_as(s, p, "modifier.add.linearArray", "Add Linear Array", "linearArray", None)),
        spec("modifier.add.circularArray", "Circular Array", |s, p| {
            add_as(s, p, "modifier.add.circularArray", "Add Circular Array", "circularArray", None)
        }),
        spec("modifier.add.gridArray", "Grid Array", |s, p| add_as(s, p, "modifier.add.gridArray", "Add Grid Array", "gridArray", None)),
        spec("modifier.add.mirror", "Mirror", |s, p| add_as(s, p, "modifier.add.mirror", "Add Mirror", "mirror", None)),
        spec("modifier.add.wallpaper", "Wallpaper", |s, p| add_as(s, p, "modifier.add.wallpaper", "Add Wallpaper", "wallpaper", None)),
        spec("modifier.add.copyToShape", "Copy to Shape", |s, p| add_as(s, p, "modifier.add.copyToShape", "Add Copy to Shape", "copyToShape", None)),
        spec("modifier.add.transformEach", "Transform Each", |s, p| {
            add_as(s, p, "modifier.add.transformEach", "Add Transform Each", "transformEach", None)
        }),
        spec("modifier.add.randomize", "Randomize", |s, p| add_as(s, p, "modifier.add.randomize", "Add Randomize", "randomize", None)),
        spec("modifier.add.resample", "Resample", |s, p| add_as(s, p, "modifier.add.resample", "Add Resample", "resample", None)),
        spec("modifier.add.trim", "Trim", |s, p| add_as(s, p, "modifier.add.trim", "Add Trim", "trim", None)),
        spec("modifier.add.tidy", "Tidy", |s, p| add_as(s, p, "modifier.add.tidy", "Add Tidy", "tidy", None)),
        spec("modifier.add.deform", "Deform", |s, p| add_as(s, p, "modifier.add.deform", "Add Deform", "deform", None)),
        spec("modifier.add.displace", "Displace", |s, p| add_as(s, p, "modifier.add.displace", "Add Displace", "displace", None)),
        spec("modifier.add.screen", "Screen", |s, p| add_as(s, p, "modifier.add.screen", "Add Screen", "screen", None)),
        spec("modifier.add.benDay", "Ben-Day Dots", |s, p| add_as(s, p, "modifier.add.benDay", "Add Ben-Day Dots", "screen", Some("benDay"))),
        spec("modifier.add.lineFill", "Line Fill", |s, p| add_as(s, p, "modifier.add.lineFill", "Add Line Fill", "screen", Some("lineFill"))),
        spec("modifier.add.color", "Color", |s, p| add_as(s, p, "modifier.add.color", "Add Color", "color", None)),
        spec("modifier.add.rainbow", "Rainbow", |s, p| add_as(s, p, "modifier.add.rainbow", "Add Rainbow", "color", Some("rainbow"))),
        spec("modifier.add.fade", "Fade", |s, p| add_as(s, p, "modifier.add.fade", "Add Fade", "color", Some("fade"))),
        spec("modifier.add.vary", "Vary", |s, p| add_as(s, p, "modifier.add.vary", "Add Vary", "color", Some("vary"))),
        spec("modifier.add.stencil", "Stencil", |s, p| add_as(s, p, "modifier.add.stencil", "Add Stencil", "stencil", None)),
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// Each command adds its kind (and preset) to the selection as `modifier.add` does, one undo
    /// step; every kind the modifiers crate knows has one.
    #[test]
    fn each_kind_command_adds_its_kind_in_one_undo_step() {
        for (kind, _) in vectorcraft_modifiers::KINDS {
            assert!(super::specs().iter().any(|c| c.id == format!("modifier.add.{kind}")), "{kind} has a command");
        }
        for c in super::specs() {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
            let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            s.execute(c.id, &json!({})).unwrap();
            let node = |s: &Session| s.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
            let stack = vectorcraft_modifiers::node_stack(&node(&s)).unwrap().unwrap();
            let want = match c.id {
                "modifier.add.benDay" | "modifier.add.lineFill" => "screen",
                "modifier.add.rainbow" | "modifier.add.fade" | "modifier.add.vary" => "color",
                id => id.trim_start_matches("modifier.add."),
            };
            assert_eq!(stack.modifiers.iter().map(|m| m.kind.id()).collect::<Vec<_>>(), [want], "{}", c.id);
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(!vectorcraft_modifiers::has_modifiers(&node(&s)), "{}: one undo step", c.id);
        }
        // A color preset fills in its shifts.
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("modifier.add.rainbow", &json!({"ids": [id]})).unwrap();
        let n = s.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let p = vectorcraft_modifiers::node_stack(&n).unwrap().unwrap().modifiers[0].kind.params();
        assert_eq!((p["hue"].as_f64(), p["chroma"].as_f64(), p["lightness"].as_f64()), (Some(100.0), Some(0.0), Some(0.0)));
        assert!(
            s.execute("modifier.add", &json!({"ids": [id], "kind": "color", "preset": "sepia"}))
                .unwrap_err()
                .to_string()
                .contains("rainbow|fade|vary")
        );
        // A preset fills in the screen's preset.
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("modifier.add.lineFill", &json!({"ids": [id]})).unwrap();
        let n = s.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let p = vectorcraft_modifiers::node_stack(&n).unwrap().unwrap().modifiers[0].kind.params();
        assert_eq!(p["spot"].as_str(), Some("line"));
        // Its own label names the undo step; another kind or preset is refused, not overwritten.
        assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Add Line Fill"));
        assert!(s.execute("modifier.add.lineFill", &json!({"ids": [id], "kind": "mirror"})).is_err());
        assert!(s.execute("modifier.add.lineFill", &json!({"ids": [id], "preset": "benDay"})).is_err());
        assert!(s.execute("modifier.add.mirror", &json!({"ids": [id], "preset": "benDay"})).is_err());
        assert!(s.execute("modifier.add.mirror", &json!({"ids": [id], "kind": "mirror"})).is_ok());
    }
}
