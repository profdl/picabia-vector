//! PICABIA: modifier stack commands (`modifier.*`).
//!
//! An object's stack lives in its `picabia.modifiers` foreign attribute (see
//! `vectorcraft_modifiers`), so save and reopen, copy and paste, duplicate and undo carry it like
//! any of the object's attributes. Every editing command is one undo step; `ids` (or `id`) default
//! to the selection and name the objects. Layers and sublayers are refused: modifiers go on
//! objects. A modifier in a stack is picked with `modifier` (its id, e.g. `"m1"`) or `index`
//! (0 = the top row, which runs first).
//!
//! An object whose modifier data is damaged refuses edits (its data is kept) until
//! `modifier.remove_all` clears it.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_modifiers::{Modifier, Stack};

use super::{CommandSpec, always, cmd, has_doc, has_selection, ok, targets};
use crate::{EngineError, Result, Session};

/// The kinds `modifier.add` takes, for messages.
const KIND_LIST: &str = "linearArray|circularArray|gridArray|mirror|transformEach|randomize|wallpaper|copyToShape|stencil";

/// Picabia's state in a [`Session`] (its `picabia` field), not saved.
#[derive(Debug, Default)]
pub(crate) struct PicabiaState {
    /// The modifier rows `modifier.copy` copied (each one modifier object), for `modifier.paste`.
    pub(crate) copied_modifiers: Vec<Value>,
    /// The laser link (`laser.*`, [`super::picabia_laser_link`]); dropping it stops a running job.
    pub(crate) laser: super::picabia_laser_link::LaserLink,
    /// What the connected laser machine reports, for the G-code commands' defaults (the laser
    /// link fills it while connected; empty: the machine profile's values are used).
    pub(crate) laser_machine: LaserMachine,
}

/// What a connected laser machine reports ([`PicabiaState::laser_machine`]). The G-code commands
/// only read it: they write text and never talk to a machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaserMachine {
    /// GRBL `$30`: `S` at full power.
    pub s_max: Option<f64>,
    /// GRBL `$130` / `$131`: the X and Y travel, mm.
    pub travel_x_mm: Option<f64>,
    pub travel_y_mm: Option<f64>,
}

impl PicabiaState {
    /// What the connected laser machine reports: read live from the laser link while it is open
    /// (its settings arrive after `laser.connect`, between commands), else
    /// [`PicabiaState::laser_machine`] (the link's last copy, cleared when it closes).
    pub(crate) fn machine_reports(&self) -> LaserMachine {
        self.laser.reports().unwrap_or_else(|| self.laser_machine.clone())
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "modifier.add",
            "Add Modifier",
            [],
            None,
            "{ids?|id?: the objects (default: the selection), kind: linearArray|circularArray|gridArray|mirror|transformEach|randomize|wallpaper|copyToShape|stencil, props?: {…the kind's params, camelCase: points, degrees, scale factors (1 = 100%); copyToShape: target (an object id; refused: the object itself, its groups or its contents, a top-level layer, or an object copying back onto it), mode (points|alongPath|populate), count, scale, align, seed; stencil (ends the stack: rows below it are ignored): its params as docs/picabia/modifiers.md lists them}, index?: where in the stack (default: the end, which runs last)} add a modifier to each object's stack, as one undo step → {modifier: the new id on the first object, added: [{id, modifier}]}",
            has_selection,
            add
        ),
        cmd!(
            "modifier.remove",
            "Remove Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id (e.g. \"m1\"), index?: 0 = the top row} remove one modifier from each object's stack, as one undo step",
            has_selection,
            remove
        ),
        cmd!(
            "modifier.move",
            "Move Modifier",
            [],
            None,
            "{ids?|id?, from: index, to: index} move a modifier up or down each object's stack, as one undo step",
            has_selection,
            move_modifier
        ),
        cmd!(
            "modifier.set",
            "Set Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, props: {…params to change, camelCase, as modifier.add takes them; enabled too; a stencil material preset fills its limits; <field>Expr: a per-copy expression on transformEach and randomize amounts (i, n, t, value; \"\" clears)}} change a modifier's params, as one undo step",
            has_selection,
            set
        ),
        cmd!(
            "modifier.toggle",
            "Toggle Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id | index?, enabled?: true|false (default: flip)} turn a modifier on or off, as one undo step",
            has_selection,
            toggle
        ),
        cmd!(
            "modifier.copy",
            "Copy Modifiers",
            [],
            None,
            "{id?: the object (default: the first selected), modifier?: id | index?: one row (without: the whole stack)} copy modifier rows for modifier.paste (not the objects' clipboard) → {copied}",
            has_doc,
            copy
        ),
        cmd!(
            "modifier.paste",
            "Paste Modifiers",
            [],
            None,
            "{ids?|id?, replace?: replace the stacks (default: add below the existing rows)} paste the rows modifier.copy copied onto each object, with fresh ids, as one undo step → {pasted}",
            has_selection,
            paste
        ),
        cmd!(
            query "modifier.list",
            "List Modifiers",
            [],
            None,
            "{id?: the object (default: the first selected)} → {id, modifiers: [{id, enabled, kind, label, …params}], instances: copies drawn (0 = drawn normally), warnings: [{modifier?, message}], error?: why the modifier data can't be read}",
            has_doc,
            list
        ),
        cmd!(
            "modifier.remove_all",
            "Remove All Modifiers",
            [],
            None,
            "{ids?|id?} remove the whole modifier stack from each object (damaged data too), as one undo step",
            has_selection,
            remove_all
        ),
        cmd!(
            "modifier.apply",
            "Apply Modifiers",
            [],
            None,
            "{ids?|id?, upTo?: index (inclusive): bake only the rows down to it; the rows after it move onto the new group and stay live} replace each object by a group of real objects, one per modifier copy (named `Name 1…n` when the object has a name), in its place, as one undo step; a stencil row stays live on the group; objects whose stack makes no copies are left alone → {ids: the groups (selected), applied: [{id, group, copies}]}",
            has_selection,
            apply::apply
        ),
        cmd!(
            "modifier.duplicate",
            "Duplicate Modifier",
            [],
            None,
            "{ids?|id?, modifier?: id | index?} put a copy of the row right below it, with a fresh id, as one undo step → {modifier: the new id on the first object, duplicated: [{id, modifier}]}",
            has_selection,
            targets::duplicate
        ),
        cmd!(
            query "modifier.targets",
            "Copy to Shape Targets",
            [],
            None,
            "{id?: the object (default: the first selected), modifier?: id | index?: a copyToShape row (also its target, valid, reason?)} → {id, targets: [{id, name, kind}] (the objects it can copy onto, in layer order)}",
            has_doc,
            targets::list_targets
        ),
        cmd!(
            query "modifier.stencilInfo",
            "Stencil Info",
            [],
            None,
            "{id?: the object (default: the first selected)} (computes the stencil when it is out of date) → {id, stencil: the stencil row's id (null when the object's stack has none), colours: [{hex, cut, area (mm², visible)}], warnings: […], stats: {cutPieces, droppedSpecks, filledHoles, cutLengthMm, areaCutMm2, islands, ties, untied, patternHeld, repeatIslands (islands per repeat cell), problems: {narrowPaper, narrowCut, smallCut, smallHole, sharpCorner, untied}}, islands: [{x, y, w, h (bounds, document space), tied (tie bars holding it), patternHeld, depth (0: in a cut-out of the sheet)}], problems: [{kind, x, y, w, h}] (at most 500), hoshi: [{x, y}] (registration hole centres), repeat: {ax, ay, bx, by} | null (the lattice when repeat-aware), cell: [{x, y} ×4] | null (the repeat cell), sheetMm: {w, h} (the whole design's sheet), tile: {w, h} | null (mm, tile output), materials: [{id, minCutWidthMm, minBridgeMm, tieWidthMm, minCornerAngle, kerfMm}] (presets), shapes (outlines gathered)}",
            has_doc,
            stencil::stencil_info
        ),
        cmd!(
            "modifier.stencilFromSelection",
            "Make Stencil",
            [],
            None,
            "{ids?|id?: the objects (default: the selection), props?: the stencil's params, as modifier.add} one object takes a stencil row itself; several are grouped first (a group named Stencil, where the front-most was), one undo step with the row → {id: the stencil object (selected), modifier}",
            has_selection,
            stencil::stencil_from_selection
        ),
        cmd!(
            query "modifier.stencilExport",
            "Export for Laser…",
            [],
            None,
            "{id?: the stencil object (default: the first selected), path?: the .svg to write (several sheets: \" 1\", \" 2\"… before the extension), data?: true: return the files as base64, dryRun?: true: only describe the sheets (nothing written or returned, settings not remembered: the dialog's summary); none of path, data, dryRun: nothing is made, the result is what the dialog opens with ({dialog: \"stencilExport\", id, params}), kerfMm? (0-2; default the stencil material's kerf, custom: the remembered one), bedWidthMm?, bedHeightMm? (10-5000 mm; 0 = no limit; a bigger sheet is split into overlapping sheets), overlapMm? (0-200), cutColour?, engraveColour?, scoreColour? (\"#rrggbb\"), label? (engrave the object's name, size, material, sheet number and THIS SIDE UP)} SVG for laser software such as LightBurn: the drawn cut (one tile with tile output), mm, kerf-compensated, inner cuts first; settings not given: the ones used last; computes the stencil when it is out of date → {id, files: [{name, path | data, sheet (\"1 of 2\"), wMm, hMm, contours, loosePieces}], sheets, warnings, params {kerfMm, bedWidthMm, bedHeightMm, overlapMm, cutColour, engraveColour, scoreColour, label} (remembered for next time)}",
            has_doc,
            laser::export
        ),
        cmd!(
            query "modifier.stencilTestCard",
            "Laser Test Card…",
            [],
            None,
            "{id?: a stencil object (default: the first selected, when it is one), material? (paper|film|card|custom; default the stencil's, else paper; custom needs a stencil: its limits), kind? (card (default): a 120 × 80 mm card of bridges, holes, slits, corners and a kerf gauge at the material's limits | powerLadder: a grid of squares each cut at its own power and speed, G-code only), format? (svg | gcode; default svg for the card, gcode for the ladder), kerfMm?, path?, data?, dryRun? (as modifier.stencilExport; none: {dialog: \"stencilTestCard\", id, material, params, format, kind, gcode?}), cutColour?, engraveColour?, scoreColour? (svg), sMax?, originXMm?, originYMm?, air?, cut?, score?, engrave? (G-code settings, as modifier.stencilGcode), powersPct?, speedsMmMin? (the ladder's columns and rows, at most 12 each), squareMm? (2-50), gapMm? (1-50), passes? (1-20)} → {files: [{name, path | data, sheet, wMm, hMm, contours, loosePieces (svg) | lines, estimatedSec, burnMm, travelMm, boundsMm {x0, y0, x1, y1}, warnings (gcode)}], sheets: 1, warnings, material, params, kind? and gcode? (gcode), ladder? {powersPct, speedsMmMin, squareMm, gapMm, passes}}",
            always,
            laser::test_card
        ),
        cmd!(
            query "modifier.stencilGcode",
            "Export G-code for Laser…",
            [],
            None,
            "{id?, path?: the .gcode to write (several sheets: \" 1\", \" 2\"… before the extension), data?, dryRun? (as modifier.stencilExport; none: {dialog: \"stencilExport\", id, format: \"gcode\", material, params, gcode}), material? (paper|film|card|custom: whose operation settings; default the stencil's), kerfMm?, bedWidthMm?, bedHeightMm? (default the machine profile's bed, less where the connected machine's travel is smaller; a bigger sheet is split), overlapMm?, label?, sMax? (S at full power; default the connected machine's $30, else the profile's), originXMm?, originYMm? (where the sheet's bottom-left sits in work coordinates, mm, ±5000; default 0, 0), air? (M8/M9; default the profile's), cut?, score?, engrave? ({enabled?, speedMmMin? (1-20000), powerPct? (0-100), passes? (1-20)}, over the material's remembered settings)} GRBL 1.1 laser mode G-code, written as text only (never sent): G21 G90, M4 dynamic power, engrave then score then cut, inner contours first, beam off (M5) at both ends, park at the origin; Y up from the sheet's bottom-left → {id, files: [{name, path | data, sheet, wMm, hMm, lines, estimatedSec, burnMm, travelMm, boundsMm {x0, y0, x1, y1}, warnings}], sheets, warnings, material, params (the laser settings used), gcode {sMax, originXMm, originYMm, air, cut, score, engrave} (op settings remembered per material in the machine profile)}",
            has_doc,
            laser_gcode::stencil_gcode
        ),
        cmd!(
            query "laser.gcode",
            "Export Design G-code for Laser…",
            [],
            None,
            "{ids?|id?: the objects (default: the selection; a single stencil object is exported as modifier.stencilGcode), kerfMm?, overlapMm?, label?, bedWidthMm?, bedHeightMm? (a single stencil object's only), path?: the .gcode to write, data?, dryRun? (as modifier.stencilExport; none: {dialog: \"laserGcode\", ids, material, gcode}), material? (paper|film|card|custom: whose operation settings; default paper), mmPerUnit? (mm per document unit, 0.001-100; default 25.4/72: a point), widthMm? (scale the objects' bounds to this width instead), sMax?, originXMm?, originYMm?, air?, cut?, score?, engrave? (as modifier.stencilGcode)} the objects' visible outlines as one G-code file (text only, never sent): mm, origin at their bounds' top-left; closed outlines cut (inner first, nearest neighbour, the largest outermost last), open paths along their line with the score settings, type as glyph outlines, modifier copies included; no kerf compensation, no sheet outline → {files: [{name, path | data, sheet (\"design\"), wMm, hMm, lines, estimatedSec, burnMm, travelMm, boundsMm, warnings}], sheets: 1, warnings, material, mmPerUnit, gcode}",
            has_doc,
            laser_design::gcode
        ),
        cmd!(
            query "laser.profile",
            "Laser Machine Profile",
            [],
            None,
            "{set?: {name?, bedWidthMm?, bedHeightMm? (10-5000), sMax? (1-100000), rapidMmMin? (10-20000), air?, checked? (the values were checked on the machine), origin? (head: work 0,0 is where the person pressed Set origin, the sheet's bottom-left (default) | machine: work coordinates are the bed's), materials?: {paper|film|card|custom: {cut?, score?, engrave?: {enabled?, speedMmMin?, powerPct?, passes?}}}} (merged into the saved profile)} → {profile {name, bedWidthMm, bedHeightMm, sMax, rapidMmMin, air, checked, origin, materials}, bedMm {w, h}, sMaxUsed, sMaxMachine | null, materialDefaults {paper|film|card|custom: {cut, score, engrave}}} (defaults: the owner's X-Carve, 584.2 × 1219.2 mm, $30 1000, rapid 3000 mm/min, no air: guesses until checked)",
            always,
            laser_gcode::profile
        ),
    ]
}

/// The objects a command edits: `ids`/`id` or the selection; layers and sublayers are refused.
fn objects(cmd: &str, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = targets(s, p).map_err(|e| super::bad(cmd, e.to_string()))?;
    if ids.is_empty() {
        return Err(super::bad(cmd, "select an object or pass `ids`"));
    }
    let doc = &s.doc()?.doc;
    for id in &ids {
        let n = doc.node(*id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
        if n.is_layer() {
            return Err(super::bad(cmd, format!("{} is a layer; modifiers go on objects", id.0)));
        }
    }
    Ok(ids)
}

/// The one object a query reads: `id` or the first selected.
fn one_object(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let id = targets(s, p).map_err(|e| super::bad(cmd, e.to_string()))?.first().copied();
    let id = id.ok_or_else(|| super::bad(cmd, "select an object or pass `id`"))?;
    s.doc()?.doc.node(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    Ok(id)
}

/// Object `id`'s stack (empty without modifier data); damaged data is refused, not overwritten.
pub(super) fn read(cmd: &str, doc: &Document, id: NodeId) -> Result<Stack> {
    let n = doc.node(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    match vectorcraft_modifiers::node_source(n) {
        None => Ok(Stack::default()),
        Some(src) => vectorcraft_modifiers::parse(src).map_err(|e| super::bad(cmd, format!("object {}: {e} (modifier.remove_all clears it)", id.0))),
    }
}

/// Store `stack` on object `id` (an empty stack removes the data).
pub(super) fn write(cmd: &str, doc: &mut Document, id: NodeId, stack: &Stack) -> Result<()> {
    let n = doc.node_mut(id).ok_or_else(|| super::bad(cmd, format!("no object with id {}", id.0)))?;
    let src = (!stack.is_empty()).then(|| vectorcraft_modifiers::to_json(stack));
    vectorcraft_modifiers::set_node_source(n, src);
    Ok(())
}

/// Read, change with `f`, and write back every object's stack, as one undo step `label`.
fn edit(cmd: &str, label: &str, s: &mut Session, p: &Value, mut f: impl FnMut(&mut Stack, NodeId) -> Result<()>) -> Result<()> {
    let ids = objects(cmd, s, p)?;
    s.edit(label, |doc, _| {
        for id in ids {
            let mut stack = read(cmd, doc, id)?;
            let before = stack.clone();
            f(&mut stack, id)?;
            // Copy-to-shape targets it sets or turns on.
            targets::check_changed(cmd, doc, id, &before, &stack)?;
            write(cmd, doc, id, &stack)?;
        }
        Ok(())
    })
}

/// A non-negative integer param `key`: `None` when absent or null.
pub(super) fn opt_index(cmd: &str, p: &Value, key: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            v.as_u64().map(|i| Some(super::index(i))).ok_or_else(|| super::bad(cmd, format!("`{key}` must be a non-negative integer, not {v}")))
        }
    }
}

fn req_index(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    opt_index(cmd, p, key)?.ok_or_else(|| super::bad(cmd, format!("`{key}` required (an index in the stack)")))
}

/// Which modifier `p` names (`modifier` id or `index`) in `stack`, on object `id`.
pub(super) fn pick(cmd: &str, p: &Value, stack: &Stack, id: NodeId) -> Result<usize> {
    let given = p.get("modifier").filter(|v| !v.is_null());
    let at = match (given, opt_index(cmd, p, "index")?) {
        (Some(m), None) => {
            let m = m.as_str().ok_or_else(|| super::bad(cmd, "`modifier` must be a modifier id such as \"m1\""))?;
            stack.position(m).ok_or_else(|| super::bad(cmd, format!("object {} has no modifier `{m}`", id.0)))?
        }
        (None, Some(i)) => i,
        (Some(_), Some(_)) => return Err(super::bad(cmd, "pass `modifier` or `index`, not both")),
        (None, None) => return Err(super::bad(cmd, "`modifier` (an id) or `index` required")),
    };
    if at >= stack.len() {
        return Err(super::bad(cmd, format!("object {} has no modifier at index {at} ({} in its stack)", id.0, stack.len())));
    }
    Ok(at)
}

pub(super) fn props_of<'a>(cmd: &str, p: &'a Value, required: bool) -> Result<Option<&'a Map<String, Value>>> {
    match p.get("props") {
        None | Some(Value::Null) if required => Err(super::bad(cmd, "`props` required")),
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(super::bad(cmd, "`props` must be an object")),
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.add";
    let kind = super::text_param(p, "kind", CMD)?.ok_or_else(|| super::bad(CMD, format!("`kind` required ({KIND_LIST})")))?;
    let mut template = Modifier::new("", kind).ok_or_else(|| super::bad(CMD, format!("unknown kind `{kind}` (one of {KIND_LIST})")))?;
    if let Some(props) = props_of(CMD, p, false)? {
        template.set_props(props).map_err(|e| super::bad(CMD, e.to_string()))?;
    }
    let at = opt_index(CMD, p, "index")?;
    let mut added = Vec::new();
    edit(CMD, "Add Modifier", s, p, |stack, id| {
        let m = stack.insert(at.unwrap_or(usize::MAX), template.clone()).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))?;
        added.push(json!({"id": id.0, "modifier": m.id}));
        Ok(())
    })?;
    let first = added.first().and_then(|a| a.get("modifier")).cloned().unwrap_or(Value::Null);
    Ok(json!({"modifier": first, "added": added}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.remove";
    edit(CMD, "Remove Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        if at < stack.modifiers.len() {
            stack.modifiers.remove(at);
        }
        Ok(())
    })?;
    ok()
}

fn move_modifier(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.move";
    let (from, to) = (req_index(CMD, p, "from")?, req_index(CMD, p, "to")?);
    edit(CMD, "Move Modifier", s, p, |stack, id| {
        let n = stack.len();
        if from >= n || to >= n {
            return Err(super::bad(CMD, format!("object {} has {n} modifiers (indexes 0..{})", id.0, n.saturating_sub(1))));
        }
        let m = stack.modifiers.remove(from);
        stack.modifiers.insert(to, m);
        Ok(())
    })?;
    ok()
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.set";
    let props = props_of(CMD, p, true)?.cloned().unwrap_or_default();
    edit(CMD, "Set Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        let m = stack.modifiers.get_mut(at).ok_or_else(|| super::bad(CMD, "no such modifier"))?;
        m.set_props(&props).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))
    })?;
    ok()
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.toggle";
    let enabled = match p.get("enabled") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| super::bad(CMD, "`enabled` must be true or false"))?),
    };
    edit(CMD, "Toggle Modifier", s, p, |stack, id| {
        let at = pick(CMD, p, stack, id)?;
        let m = stack.modifiers.get_mut(at).ok_or_else(|| super::bad(CMD, "no such modifier"))?;
        m.enabled = enabled.unwrap_or(!m.enabled);
        Ok(())
    })?;
    ok()
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.copy";
    let id = one_object(CMD, s, p)?;
    let stack = read(CMD, &s.doc()?.doc, id)?;
    let picked = p.get("modifier").is_some_and(|v| !v.is_null()) || p.get("index").is_some_and(|v| !v.is_null());
    let rows: Vec<Value> = if picked {
        let at = pick(CMD, p, &stack, id)?;
        stack.modifiers.get(at).map(Modifier::to_value).into_iter().collect()
    } else {
        stack.modifiers.iter().map(Modifier::to_value).collect()
    };
    if rows.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), format!("object {} has no modifiers", id.0)));
    }
    let copied = rows.len();
    s.picabia.copied_modifiers = rows;
    Ok(json!({"copied": copied}))
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.paste";
    if s.picabia.copied_modifiers.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "no copied modifiers".into()));
    }
    let rows = s.picabia.copied_modifiers.iter().map(Modifier::from_value).collect::<std::result::Result<Vec<_>, _>>();
    let rows = rows.map_err(|e| super::bad(CMD, e.to_string()))?;
    let replace = super::switch_or(p, "replace", false, CMD)?;
    edit(CMD, "Paste Modifiers", s, p, |stack, id| {
        if replace {
            stack.modifiers.clear();
        }
        for m in &rows {
            stack.insert(usize::MAX, m.clone()).map_err(|e| super::bad(CMD, format!("object {}: {e}", id.0)))?;
        }
        Ok(())
    })?;
    Ok(json!({"pasted": rows.len()}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.list";
    let id = one_object(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let node = doc.node(id).ok_or_else(|| super::bad(CMD, format!("no object with id {}", id.0)))?;
    let mut out = json!({"id": id.0, "modifiers": [], "instances": 0, "warnings": []});
    match vectorcraft_modifiers::node_stack(node) {
        None => {}
        Some(Err(e)) => out["error"] = json!(e.to_string()),
        Some(Ok(stack)) => {
            let rows: Vec<Value> = stack
                .modifiers
                .iter()
                .map(|m| {
                    let mut v = m.to_value();
                    if let Some(o) = v.as_object_mut() {
                        o.insert("label".into(), json!(m.label()));
                    }
                    v
                })
                .collect();
            out["modifiers"] = json!(rows);
            let targets = vectorcraft_modifiers::DocTargets::new(doc, id);
            let o = vectorcraft_modifiers::evaluate_with(&stack, vectorcraft_modifiers::node_bounds(node), &targets);
            out["instances"] = json!(o.instances.len());
            out["warnings"] = json!(o.warnings.iter().map(|w| json!({"modifier": w.modifier, "message": w.message})).collect::<Vec<_>>());
        }
    }
    Ok(out)
}

fn remove_all(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "modifier.remove_all";
    let ids = objects(CMD, s, p)?;
    let doc = &s.doc()?.doc;
    let with: Vec<NodeId> = ids.into_iter().filter(|id| doc.node(*id).is_some_and(vectorcraft_modifiers::has_modifiers)).collect();
    // Nothing to remove: no undo step.
    if with.is_empty() {
        return ok();
    }
    s.edit("Remove All Modifiers", |doc, _| {
        for id in with {
            if let Some(n) = doc.node_mut(id) {
                vectorcraft_modifiers::set_node_source(n, None);
            }
        }
        Ok(())
    })?;
    ok()
}

mod apply;
mod laser;
mod laser_design;
pub mod laser_gcode;
pub(crate) mod laser_run;
mod stencil;
mod targets;
pub use targets::{check_target, target_choices};
pub(crate) use targets::{remap_copies, remap_pasted};
#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod laser_tests;
#[cfg(test)]
mod stencil_tests;
#[cfg(test)]
mod targets_tests;
#[cfg(test)]
mod tests;
