//! An object's physics settings: the model, its lenient parser (clamps out-of-range values and
//! ignores wrong types, each with a warning) and the foreign attribute that stores it.
//!
//! Ported from tlprocedural's `PhysicsSettings` (`src/types/modifiers.ts`) with its defaults
//! (`PHYSICS_DEFAULTS`, `modifierConfig.ts`) and ranges (`PHYSICS_SCHEMA`, `modifierSchemas.ts`).

use serde_json::{Map, Value, json};
use vectorcraft_doc::{ForeignAttribute, Node};

use crate::{MAX_SOURCE_BYTES, PHYSICS_KEY, PHYSICS_NS, PHYSICS_PREFIX, SETTINGS_VERSION};

/// How a body moves.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyType {
    /// Falls, collides and is moved by the simulation.
    Dynamic,
    /// Never moves; others collide with it.
    Static,
    /// Moved only by its owner (phase 1 has no clock, so it stays put like a static body).
    Kinematic,
    /// A kind this version doesn't know (a newer Picabia's, e.g. a future `"soft"`): kept on a
    /// round trip and simulated as a static body.
    Other(String),
}

impl BodyType {
    pub fn as_str(&self) -> &str {
        match self {
            BodyType::Dynamic => "dynamic",
            BodyType::Static => "static",
            BodyType::Kinematic => "kinematic",
            BodyType::Other(s) => s,
        }
    }
}

/// Collider fidelity (tlprocedural's `ColliderFidelity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColliderType {
    /// tlprocedural's rule by vertex count: ≤ 16 the convex hull, ≤ 64 the hull of the outline
    /// simplified to 32 points, more: the outline simplified to 64 points, exact (see `shape.rs`).
    Auto,
    /// The outline itself (dynamic: split into convex parts; static: its edges).
    Precise,
    /// The convex hull of the whole object.
    Fast,
    /// The object's geometric bounding box.
    Box,
}

impl ColliderType {
    pub fn as_str(self) -> &'static str {
        match self {
            ColliderType::Auto => "auto",
            ColliderType::Precise => "precise",
            ColliderType::Fast => "fast",
            ColliderType::Box => "box",
        }
    }
}

/// One object's physics settings. Units: masses in kg, damping per second; lengths don't appear
/// (see the crate docs for the scale).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Off: the object takes no part in a simulation (the settings are kept).
    pub enabled: bool,
    pub body_type: BodyType,
    pub collider: ColliderType,
    /// Gravity multiplier, 0–2.
    pub gravity_scale: f64,
    /// Bounciness, 0–1.
    pub restitution: f64,
    /// Friction coefficient, 0–1.
    pub friction: f64,
    /// Extra mass added to the body's own (from its area), kg, 0–10,000 (0 = the area's only).
    pub mass: f64,
    /// Rotation resistance, 0–20.
    pub angular_damping: f64,
    /// Movement resistance, 0–20.
    pub linear_damping: f64,
    /// Detects overlaps but takes no contact forces: passes through everything.
    pub sensor: bool,
    /// Fields this version doesn't know, kept on a round trip.
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    /// tlprocedural's `PHYSICS_DEFAULTS`.
    fn default() -> Self {
        Settings {
            enabled: true,
            body_type: BodyType::Dynamic,
            collider: ColliderType::Auto,
            gravity_scale: 1.2,
            restitution: 0.35,
            friction: 0.4,
            mass: 1.0,
            angular_damping: 0.02,
            linear_damping: 0.0,
            sensor: false,
            extra: Map::new(),
        }
    }
}

/// The numeric settings: `(key, min, max)`.
pub const RANGES: [(&str, f64, f64); 6] = [
    ("gravityScale", 0.0, 2.0),
    ("restitution", 0.0, 1.0),
    ("friction", 0.0, 1.0),
    ("mass", 0.0, 10_000.0),
    ("angularDamping", 0.0, 20.0),
    ("linearDamping", 0.0, 20.0),
];

/// The settings' keys, as JSON and the commands name them.
pub const KEYS: [&str; 11] =
    ["enabled", "bodyType", "colliderType", "gravityScale", "restitution", "friction", "mass", "angularDamping", "linearDamping", "sensor", "v"];

impl Settings {
    /// The settings as stored (camelCase, every field, unknown ones kept).
    pub fn to_value(&self) -> Value {
        let mut o = self.extra.clone();
        let known = json!({
            "v": SETTINGS_VERSION,
            "enabled": self.enabled,
            "bodyType": self.body_type.as_str(),
            "colliderType": self.collider.as_str(),
            "gravityScale": self.gravity_scale,
            "restitution": self.restitution,
            "friction": self.friction,
            "mass": self.mass,
            "angularDamping": self.angular_damping,
            "linearDamping": self.linear_damping,
            "sensor": self.sensor,
        });
        if let Value::Object(k) = known {
            o.extend(k);
        }
        Value::Object(o)
    }

    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    /// Settings from a stored object: defaults for missing fields, out-of-range numbers clamped,
    /// wrong types and unknown kinds ignored, each with a warning; unknown fields kept.
    pub fn from_value(v: &Value) -> Result<(Settings, Vec<String>), String> {
        let o = v.as_object().ok_or("physics settings must be a JSON object")?;
        let mut s = Settings::default();
        let mut warnings = s.apply(o, true);
        // tlprocedural's old `gravity` field, read as `gravityScale` when that is absent.
        if !o.contains_key("gravityScale")
            && let Some(g) = o.get("gravity")
        {
            s.extra.remove("gravity");
            let mut m = Map::new();
            m.insert("gravityScale".into(), g.clone());
            warnings.extend(s.apply(&m, false));
        }
        Ok((s, warnings))
    }

    /// Change the settings `props` names (camelCase), leniently: a value of the wrong type is
    /// ignored, a number out of range is clamped, an unknown body type is kept (simulated as
    /// static), an unknown collider type falls back to auto, each with a warning. Unknown keys
    /// are kept in [`Settings::extra`] with `keep_unknown`, else ignored with a warning.
    pub fn apply(&mut self, props: &Map<String, Value>, keep_unknown: bool) -> Vec<String> {
        let mut w = Vec::new();
        for (k, v) in props {
            match k.as_str() {
                "v" => {}
                "enabled" | "sensor" => match v.as_bool() {
                    Some(b) if k == "enabled" => self.enabled = b,
                    Some(b) => self.sensor = b,
                    None => w.push(format!("`{k}` must be true or false, not {v}: ignored")),
                },
                "bodyType" => match v.as_str() {
                    Some("dynamic") => self.body_type = BodyType::Dynamic,
                    Some("static") => self.body_type = BodyType::Static,
                    Some("kinematic") => self.body_type = BodyType::Kinematic,
                    Some(other) if !other.is_empty() && other.len() <= 64 => {
                        w.push(format!("body type `{other}` is not simulated by this version: the object stays put (a static body)"));
                        self.body_type = BodyType::Other(other.to_string());
                    }
                    _ => w.push(format!("`bodyType` must be dynamic, static or kinematic, not {v}: ignored")),
                },
                "colliderType" => match v.as_str() {
                    Some("auto") => self.collider = ColliderType::Auto,
                    Some("precise") => self.collider = ColliderType::Precise,
                    Some("fast") => self.collider = ColliderType::Fast,
                    Some("box") => self.collider = ColliderType::Box,
                    _ => {
                        w.push(format!("`colliderType` must be auto, precise, fast or box, not {v}: auto used"));
                        self.collider = ColliderType::Auto;
                    }
                },
                _ => {
                    if let Some(&(key, lo, hi)) = RANGES.iter().find(|r| r.0 == k) {
                        match v.as_f64().filter(|x| x.is_finite()) {
                            Some(x) => {
                                let c = x.clamp(lo, hi);
                                if c != x {
                                    w.push(format!("`{key}` {x} is outside {lo}–{hi}: {c} used"));
                                }
                                *self.number_mut(key) = c;
                            }
                            None => w.push(format!("`{key}` must be a number, not {v}: ignored")),
                        }
                    } else if keep_unknown {
                        self.extra.insert(k.clone(), v.clone());
                    } else {
                        w.push(format!("unknown physics setting `{k}`: ignored"));
                    }
                }
            }
        }
        w
    }

    fn number_mut(&mut self, key: &str) -> &mut f64 {
        match key {
            "gravityScale" => &mut self.gravity_scale,
            "restitution" => &mut self.restitution,
            "friction" => &mut self.friction,
            "mass" => &mut self.mass,
            "angularDamping" => &mut self.angular_damping,
            _ => &mut self.linear_damping,
        }
    }
}

/// [`Settings::from_value`] of JSON text (at most [`MAX_SOURCE_BYTES`]).
pub fn parse(src: &str) -> Result<(Settings, Vec<String>), String> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(format!("physics settings are too large ({} bytes, at most {MAX_SOURCE_BYTES})", src.len()));
    }
    let v: Value = serde_json::from_str(src).map_err(|e| format!("physics settings are not valid JSON: {e}"))?;
    Settings::from_value(&v)
}

/// `node`'s settings source: its `picabia.physics` foreign attribute, if any.
pub fn node_source(node: &Node) -> Option<&str> {
    let attrs = node.extras().attrs.as_deref()?;
    attrs.foreign.iter().find(|a| a.ns == PHYSICS_NS && a.name == PHYSICS_KEY).map(|a| a.value.as_str())
}

/// Whether `node` has physics settings (an attribute lookup; doesn't parse).
pub fn has_physics(node: &Node) -> bool {
    node_source(node).is_some()
}

/// `node`'s settings: `None` without any, `Some(Err)` when they can't be read at all.
pub fn node_settings(node: &Node) -> Option<Result<(Settings, Vec<String>), String>> {
    node_source(node).map(parse)
}

/// Store (`Some`) or remove (`None`) `node`'s settings. Other foreign attributes are kept.
pub fn set_node_settings(node: &mut Node, settings: Option<&Settings>) {
    if settings.is_none() && !has_physics(node) {
        return;
    }
    let src = settings.map(Settings::to_json);
    node.edit_attrs(|a| {
        let at = a.foreign.iter().position(|f| f.ns == PHYSICS_NS && f.name == PHYSICS_KEY);
        match (src, at) {
            (Some(value), Some(i)) => {
                if let Some(f) = a.foreign.get_mut(i) {
                    f.value = value;
                }
            }
            (Some(value), None) => {
                a.foreign.push(ForeignAttribute { ns: PHYSICS_NS.into(), prefix: PHYSICS_PREFIX.into(), name: PHYSICS_KEY.into(), value });
            }
            (None, Some(i)) => {
                a.foreign.remove(i);
            }
            (None, None) => {}
        }
    });
}
