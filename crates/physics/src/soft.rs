//! Soft-body settings (phase 2, PIC31.1): the `soft` object of an object whose `bodyType` is
//! `"soft"`, its presets, the fields that override them, and pins.
//!
//! Stored as the preset and only the fields changed from it, so a later tuning of a preset reaches
//! every object that didn't override it:
//!
//! ```json
//! {"bodyType":"soft","soft":{"preset":"cloth","stiffness":0.8,"pins":[{"u":0,"v":0.5}]}}
//! ```
//!
//! The fields are unitless 0–1 amounts (mapped to rapier's spring frequencies and damping ratios
//! in [`SoftParams`]), except `resolution` (the particle spacing, points) and `selfCollide`.
//! Parsing is lenient, as the rest of the settings: clamps and ignored values with warnings,
//! unknown fields and preset kinds kept on a round trip.

use serde_json::{Map, Value, json};

/// The most pins one object keeps (more are left out, with a warning). At most about 90 bytes a
/// pin, so the settings stay well within [`crate::MAX_SOURCE_BYTES`] and always read back.
pub const MAX_PINS: usize = 64;
/// Pin coordinates (fractions of the object's bounds) are clamped to ±this.
pub const MAX_PIN_UV: f64 = 4.0;

/// A soft-body preset: what kind of body the object becomes and the starting values of its fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Preset {
    /// A strip seen edge-on: a line of particles along the object's long axis.
    Cloth,
    /// The outline filled with cells, soft and wobbly.
    Jelly,
    /// The same model as jelly, stiff and bouncy.
    Rubber,
    /// A preset this version doesn't know (a later Picabia's): kept on a round trip, simulated as
    /// jelly, with a warning.
    Other(String),
}

impl Preset {
    pub fn as_str(&self) -> &str {
        match self {
            Preset::Cloth => "cloth",
            Preset::Jelly => "jelly",
            Preset::Rubber => "rubber",
            Preset::Other(s) => s,
        }
    }

    fn parse(s: &str) -> Option<Preset> {
        match s {
            "cloth" => Some(Preset::Cloth),
            "jelly" => Some(Preset::Jelly),
            "rubber" => Some(Preset::Rubber),
            other if !other.is_empty() && other.len() <= 64 => Some(Preset::Other(other.to_string())),
            _ => None,
        }
    }

    /// Whether the body is a strip (cloth) rather than filled cells.
    pub fn is_strip(&self) -> bool {
        *self == Preset::Cloth
    }

    /// The preset's values for every field.
    pub fn defaults(&self) -> SoftFields {
        match self {
            // Cloth: stretches little, bends easily; no self-contact by default (PIC31.0: a
            // self-colliding 400-particle strip cost 62 ms a step).
            Preset::Cloth => SoftFields { stiffness: 0.85, bend: 0.1, shape_keep: 0.0, damping: 0.3, resolution: 6.0, self_collide: false },
            Preset::Jelly | Preset::Other(_) => {
                SoftFields { stiffness: 0.3, bend: 0.0, shape_keep: 0.4, damping: 0.1, resolution: 12.0, self_collide: false }
            }
            Preset::Rubber => SoftFields { stiffness: 0.85, bend: 0.0, shape_keep: 0.85, damping: 0.45, resolution: 12.0, self_collide: false },
        }
    }
}

/// Every field's value (a preset's, or with overrides applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftFields {
    /// Resistance to stretching, 0–1.
    pub stiffness: f64,
    /// Resistance to bending (cloth), 0–1.
    pub bend: f64,
    /// Pull back to its own shape (jelly, rubber), 0–1.
    pub shape_keep: f64,
    /// How quickly wobble dies out, 0–1.
    pub damping: f64,
    /// Particle spacing, points ([`RESOLUTION_RANGE`]).
    pub resolution: f64,
    /// Folds rest on themselves (costly: off by default).
    pub self_collide: bool,
}

/// `resolution`'s range, points.
pub const RESOLUTION_RANGE: (f64, f64) = (2.0, 100.0);

/// The 0–1 fields: `(key, min, max)`, then resolution.
pub const SOFT_RANGES: [(&str, f64, f64); 5] = [
    ("stiffness", 0.0, 1.0),
    ("bend", 0.0, 1.0),
    ("shapeKeep", 0.0, 1.0),
    ("damping", 0.0, 1.0),
    ("resolution", RESOLUTION_RANGE.0, RESOLUTION_RANGE.1),
];

/// A pinned point: where on the object (fractions of its geometric bounds, `u` across and `v`
/// down, so it survives moving, scaling and editing the path), and an optional object it is
/// attached to (it moves with that body; without one it stays fixed in place). At build time each
/// pin takes the nearest particle.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub u: f64,
    pub v: f64,
    /// The id of the object the point is attached to.
    pub attach_to: Option<u64>,
    /// Fields this version doesn't know, kept on a round trip.
    pub extra: Map<String, Value>,
}

impl Pin {
    /// A pin at `u`, `v`, attached to `attach_to` when given.
    pub fn new(u: f64, v: f64, attach_to: Option<u64>) -> Pin {
        Pin { u, v, attach_to, extra: Map::new() }
    }
}

impl Pin {
    fn to_value(&self) -> Value {
        let mut o = self.extra.clone();
        o.insert("u".into(), json!(self.u));
        o.insert("v".into(), json!(self.v));
        if let Some(id) = self.attach_to {
            o.insert("attachTo".into(), json!(id));
        }
        Value::Object(o)
    }

    fn from_value(v: &Value, w: &mut Vec<String>) -> Option<Pin> {
        let o = v.as_object()?;
        let coord = |k: &str, w: &mut Vec<String>| -> Option<f64> {
            let x = o.get(k)?.as_f64().filter(|x| x.is_finite())?;
            let c = x.clamp(-MAX_PIN_UV, MAX_PIN_UV);
            if c != x {
                w.push(format!("pin `{k}` {x} is outside ±{MAX_PIN_UV}: {c} used"));
            }
            Some(c)
        };
        let (Some(u), Some(vv)) = (coord("u", w), coord("v", w)) else {
            w.push(format!("a pin needs numbers `u` and `v`, not {v}: left out"));
            return None;
        };
        let attach_to = match o.get("attachTo") {
            None | Some(Value::Null) => None,
            Some(a) => match a.as_u64() {
                Some(id) => Some(id),
                None => {
                    w.push(format!("pin `attachTo` must be an object id, not {a}: pinned in place"));
                    None
                }
            },
        };
        let extra = o.iter().filter(|(k, _)| !matches!(k.as_str(), "u" | "v" | "attachTo")).map(|(k, v)| (k.clone(), v.clone())).collect();
        Some(Pin { u, v: vv, attach_to, extra })
    }
}

/// An object's soft settings: the preset, the fields set on the object (each `None` follows the
/// preset), pins, and fields this version doesn't know.
#[derive(Clone, Debug, PartialEq)]
pub struct Soft {
    pub preset: Preset,
    pub stiffness: Option<f64>,
    pub bend: Option<f64>,
    pub shape_keep: Option<f64>,
    pub damping: Option<f64>,
    pub resolution: Option<f64>,
    pub self_collide: Option<bool>,
    pub pins: Vec<Pin>,
    pub extra: Map<String, Value>,
}

impl Default for Soft {
    fn default() -> Self {
        Soft::of(Preset::Jelly)
    }
}

impl Soft {
    /// The preset with no overrides and no pins.
    pub fn of(preset: Preset) -> Soft {
        Soft {
            preset,
            stiffness: None,
            bend: None,
            shape_keep: None,
            damping: None,
            resolution: None,
            self_collide: None,
            pins: Vec::new(),
            extra: Map::new(),
        }
    }

    /// Every field: the overrides over the preset's values.
    pub fn fields(&self) -> SoftFields {
        let d = self.preset.defaults();
        SoftFields {
            stiffness: self.stiffness.unwrap_or(d.stiffness),
            bend: self.bend.unwrap_or(d.bend),
            shape_keep: self.shape_keep.unwrap_or(d.shape_keep),
            damping: self.damping.unwrap_or(d.damping),
            resolution: self.resolution.unwrap_or(d.resolution),
            self_collide: self.self_collide.unwrap_or(d.self_collide),
        }
    }

    /// The settings as stored: the preset, the overrides only, the pins, unknown fields.
    pub fn to_value(&self) -> Value {
        let mut o = self.extra.clone();
        o.insert("preset".into(), json!(self.preset.as_str()));
        for (k, v) in [
            ("stiffness", self.stiffness),
            ("bend", self.bend),
            ("shapeKeep", self.shape_keep),
            ("damping", self.damping),
            ("resolution", self.resolution),
        ] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        if let Some(b) = self.self_collide {
            o.insert("selfCollide".into(), json!(b));
        }
        if !self.pins.is_empty() {
            o.insert("pins".into(), Value::Array(self.pins.iter().map(Pin::to_value).collect()));
        }
        Value::Object(o)
    }

    /// Settings from a stored `soft` object, leniently (see [`Soft::apply`]).
    pub fn from_value(v: &Value) -> (Soft, Vec<String>) {
        let mut s = Soft::default();
        let w = match v.as_object() {
            Some(o) => s.apply(o, true),
            None => vec![format!("`soft` must be an object, not {v}: the jelly preset used")],
        };
        (s, w)
    }

    /// Change the fields `props` names: a number is clamped to its range, `null` drops the
    /// override (back to the preset's value), a wrong type is ignored, each with a warning.
    /// `preset` keeps the overrides; `pins` replaces the pins. Unknown keys are kept with
    /// `keep_unknown`, else ignored with a warning.
    pub fn apply(&mut self, props: &Map<String, Value>, keep_unknown: bool) -> Vec<String> {
        let mut w = Vec::new();
        for (k, v) in props {
            match k.as_str() {
                "preset" => match v.as_str().and_then(Preset::parse) {
                    Some(p) => {
                        if let Preset::Other(name) = &p {
                            w.push(format!("soft preset `{name}` is not known to this version: simulated as jelly"));
                        }
                        self.preset = p;
                    }
                    None => w.push(format!("`preset` must be cloth, jelly or rubber, not {v}: ignored")),
                },
                "selfCollide" => match v {
                    Value::Null => self.self_collide = None,
                    Value::Bool(b) => self.self_collide = Some(*b),
                    _ => w.push(format!("`selfCollide` must be true or false, not {v}: ignored")),
                },
                "pins" => match v {
                    Value::Null => self.pins.clear(),
                    Value::Array(a) => {
                        let read: Vec<Pin> = a.iter().filter_map(|p| Pin::from_value(p, &mut w)).collect();
                        if read.len() > MAX_PINS {
                            w.push(format!("{} pins (at most {MAX_PINS}): the last {} left out", read.len(), read.len() - MAX_PINS));
                        }
                        self.pins = read.into_iter().take(MAX_PINS).collect();
                    }
                    _ => w.push(format!("`pins` must be a list, not {v}: ignored")),
                },
                _ => {
                    if let Some(&(key, lo, hi)) = SOFT_RANGES.iter().find(|r| r.0 == k) {
                        if v.is_null() {
                            *self.number_mut(key) = None;
                            continue;
                        }
                        match v.as_f64().filter(|x| x.is_finite()) {
                            Some(x) => {
                                let c = x.clamp(lo, hi);
                                if c != x {
                                    w.push(format!("`{key}` {x} is outside {lo}–{hi}: {c} used"));
                                }
                                *self.number_mut(key) = Some(c);
                            }
                            None => w.push(format!("`{key}` must be a number, not {v}: ignored")),
                        }
                    } else if keep_unknown {
                        self.extra.insert(k.clone(), v.clone());
                    } else {
                        w.push(format!("unknown soft setting `{k}`: ignored"));
                    }
                }
            }
        }
        w
    }

    fn number_mut(&mut self, key: &str) -> &mut Option<f64> {
        match key {
            "stiffness" => &mut self.stiffness,
            "bend" => &mut self.bend,
            "shapeKeep" => &mut self.shape_keep,
            "damping" => &mut self.damping,
            _ => &mut self.resolution,
        }
    }

    /// Whether going from `self` to `new` needs a new mesh (the body restarts from its rest pose):
    /// the kind of body, the particle spacing, self-contact or the pins changed. Other fields are
    /// material and apply to the running world.
    pub fn needs_new_mesh(&self, new: &Soft) -> bool {
        let (a, b) = (self.fields(), new.fields());
        self.preset.is_strip() != new.preset.is_strip() || a.resolution != b.resolution || a.self_collide != b.self_collide || self.pins != new.pins
    }
}

/// rapier's numbers for a soft body's fields (the world's units: metres, seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftParams {
    /// Natural frequency of the structural edges, Hz.
    pub edge_hz: f32,
    /// Natural frequency of the bending edges, Hz.
    pub bend_hz: f32,
    /// Natural frequency of shape matching, Hz; 0: off.
    pub shape_hz: f32,
    /// Natural frequency of the cells' area constraints, Hz.
    pub volume_hz: f32,
    /// Damping ratio of every spring family.
    pub damping_ratio: f32,
    /// Particle spacing, points.
    pub spacing: f64,
    pub self_collide: bool,
}

impl SoftFields {
    /// The rapier numbers: stiffness 0–1 maps to 1–60 Hz, bend to 0.2–20 Hz, shape memory to 0
    /// (off) or 0.5–20 Hz (each squared first, so the low end is finely graded), damping to a
    /// damping ratio of 0.05–1. A value that isn't finite counts as 0.
    pub fn params(&self) -> SoftParams {
        let unit = |x: f64| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
        let sq = |x: f64| unit(x) * unit(x);
        let edge = 1.0 + 59.0 * sq(self.stiffness);
        SoftParams {
            edge_hz: edge as f32,
            bend_hz: (0.2 + 19.8 * sq(self.bend)) as f32,
            shape_hz: if unit(self.shape_keep) <= 0.0 { 0.0 } else { (0.5 + 19.5 * sq(self.shape_keep)) as f32 },
            volume_hz: edge as f32,
            damping_ratio: (0.05 + 0.95 * unit(self.damping)) as f32,
            spacing: if self.resolution.is_finite() { self.resolution.clamp(RESOLUTION_RANGE.0, RESOLUTION_RANGE.1) } else { RESOLUTION_RANGE.1 },
            self_collide: self.self_collide,
        }
    }
}
