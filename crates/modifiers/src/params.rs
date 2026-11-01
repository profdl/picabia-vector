//! Picabia parameters: named numbers and texts kept with the document, which modifier fields bind
//! to ([`crate::bind`]) and binding expressions read by name.
//!
//! They are the document's plug-in data under [`PARAMS_ID`] (VectorCraft's `Document::unknown`,
//! `pluginData`), so the native file, undo and every copy of the document carry them, and a
//! VectorCraft without Picabia keeps them untouched:
//!
//! ```json
//! {"v":1,"params":[{"name":"copies","type":"number","value":6,"min":1,"max":24,"step":1},
//!                  {"name":"label","type":"text","value":"A","source":"label"}]}
//! ```
//!
//! # Data sets
//! A parameter's `source` names one of VectorCraft's variables (Window ▸ Variables): while a data
//! set is active and holds a value for it, that value is the parameter's (a number parameter reads
//! the text as a number, clamped to its range). So VectorCraft's data sets are rows of parameter
//! values: select one and every object bound to those parameters redraws.
//!
//! # Names
//! A name is a letter or `_` then letters, digits and `_` ([`MAX_NAME_LEN`] at most), not one an
//! expression already uses (`i`, `n`, `t`, `value`, `pi`, the functions). Expressions ignore case,
//! so names are unique ignoring case and are looked up in lowercase.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{DataValue, Document};

use crate::expr::Names;

/// The plug-in data id parameters are kept under.
pub const PARAMS_ID: &str = "picabia.parameters";
/// The format version written in `"v"`.
pub const PARAMS_VERSION: u64 = 1;
/// The most parameters a document holds.
pub const MAX_PARAMS: usize = 256;
/// The longest parameter name (bytes).
pub const MAX_NAME_LEN: usize = 32;
/// The longest text value (bytes; longer texts are cut at a character boundary).
pub const MAX_TEXT_LEN: usize = 1024;
/// Number values, minimums and maximums are kept within ±this.
pub const MAX_PARAM_VALUE: f64 = 1.0e9;

/// A parameter's type and value.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    /// A number within `min..=max`; the Variables panel's slider moves in `step`s (0: any).
    Number {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    },
    Text {
        value: String,
    },
}

/// One parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub kind: ParamKind,
    /// The VectorCraft variable whose value in the active data set this parameter takes.
    pub source: Option<String>,
}

/// The document's parameters, in panel order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parameters {
    pub list: Vec<Parameter>,
}

/// A resolved value: what bindings read.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Number(f64),
    Text(String),
}

impl ParamValue {
    /// The value as a number: a text that reads as a finite number counts.
    pub fn number(&self) -> Result<f64, String> {
        match self {
            ParamValue::Number(v) => Ok(*v),
            ParamValue::Text(s) => s.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("is the text {s:?}, not a number")),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            ParamValue::Number(v) => json!(v),
            ParamValue::Text(s) => json!(s),
        }
    }
}

/// `s` cut to at most [`MAX_TEXT_LEN`] bytes at a character boundary.
fn cut(s: &str) -> String {
    if s.len() <= MAX_TEXT_LEN {
        return s.to_string();
    }
    let mut end = MAX_TEXT_LEN;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default().to_string()
}

/// Why `name` can't name a parameter, if it can't (its form only; uniqueness is
/// [`Parameters::check_new_name`]'s).
pub fn check_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return Err("a parameter needs a name".into()) };
    if name.len() > MAX_NAME_LEN {
        return Err(format!("keep parameter names to {MAX_NAME_LEN} characters"));
    }
    if !(first.is_ascii_alphabetic() || first == '_') || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("`{name}` isn't a name: start with a letter or _, then letters, digits or _"));
    }
    if crate::expr::reserved(name) {
        return Err(format!("`{name}` already means something in expressions (i, n, t, value, pi and the functions)"));
    }
    Ok(())
}

fn finite_in(v: &Value, key: &str) -> Result<f64, String> {
    crate::lenient::finite(v)
        .filter(|f| f.abs() <= MAX_PARAM_VALUE)
        .ok_or_else(|| format!("`{key}` must be a number within ±{MAX_PARAM_VALUE:e}, got {}", crate::model::short(v)))
}

impl Parameter {
    /// A new parameter from `props` (`type`: number (default) | text, `value`, `min`, `max`,
    /// `step`, `source`). A number's range defaults to 0–100 widened to take its value, its step
    /// to 1. Refused: a bad name, unknown keys, wrong types, `min` > `max`, a value outside the
    /// range given.
    pub fn new(name: &str, props: &Map<String, Value>) -> Result<Parameter, String> {
        check_name(name)?;
        let ty = match props.get("type") {
            None | Some(Value::Null) => "number",
            Some(Value::String(s)) if s == "number" || s == "text" => s.as_str(),
            Some(v) => return Err(format!("`type` must be number or text, got {}", crate::model::short(v))),
        };
        let kind = if ty == "text" {
            ParamKind::Text { value: String::new() }
        } else {
            let value = match props.get("value") {
                None | Some(Value::Null) => 0.0,
                Some(v) => finite_in(v, "value")?,
            };
            ParamKind::Number { value, min: value.min(0.0), max: value.max(100.0), step: 1.0 }
        };
        let mut p = Parameter { name: name.to_string(), kind, source: None };
        let rest: Map<String, Value> = props.iter().filter(|(k, _)| k.as_str() != "type").map(|(k, v)| (k.clone(), v.clone())).collect();
        p.set(&rest)?;
        Ok(p)
    }

    /// Change `props` (`value`, `min`, `max`, `step`, `source`: a variable name, or `null`/"" for
    /// none). A number's value is clamped into its range when the range moves; a value given
    /// outside the range is refused. Leaves the parameter unchanged when refused.
    pub fn set(&mut self, props: &Map<String, Value>) -> Result<(), String> {
        let mut next = self.clone();
        for (k, v) in props {
            match (k.as_str(), &mut next.kind) {
                ("source", _) => {
                    next.source = match v {
                        Value::Null => None,
                        Value::String(s) if s.trim().is_empty() => None,
                        Value::String(s) if s.len() <= 256 => Some(s.clone()),
                        _ => return Err(format!("`source` must be a variable name (at most 256 bytes) or null, got {}", crate::model::short(v))),
                    };
                }
                ("value", ParamKind::Text { value }) => match v {
                    Value::String(s) => *value = cut(s),
                    Value::Number(n) => *value = n.to_string(),
                    Value::Bool(b) => *value = b.to_string(),
                    _ => return Err(format!("`value` must be a text, got {}", crate::model::short(v))),
                },
                ("value", ParamKind::Number { value, .. }) => *value = finite_in(v, "value")?,
                ("min", ParamKind::Number { min, .. }) => *min = finite_in(v, "min")?,
                ("max", ParamKind::Number { max, .. }) => *max = finite_in(v, "max")?,
                ("step", ParamKind::Number { step, .. }) => {
                    let s = finite_in(v, "step")?;
                    if s < 0.0 {
                        return Err("`step` can't be negative (0 = any value)".into());
                    }
                    *step = s;
                }
                ("min" | "max" | "step", ParamKind::Text { .. }) => return Err(format!("a text parameter has no `{k}`")),
                _ => return Err(format!("parameters have no `{k}` (they take value, min, max, step, source)")),
            }
        }
        if let ParamKind::Number { value, min, max, .. } = &mut next.kind {
            if *min > *max {
                return Err(format!("`min` ({min}) is more than `max` ({max})"));
            }
            if props.contains_key("value") && (*value < *min || *value > *max) {
                return Err(format!("{value} is outside {min}–{max}"));
            }
            *value = value.clamp(*min, *max);
        }
        *self = next;
        Ok(())
    }

    /// The type's name, as `param.add` takes it.
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            ParamKind::Number { .. } => "number",
            ParamKind::Text { .. } => "text",
        }
    }

    /// Its own value (not the data set's).
    pub fn own_value(&self) -> ParamValue {
        match &self.kind {
            ParamKind::Number { value, .. } => ParamValue::Number(*value),
            ParamKind::Text { value } => ParamValue::Text(value.clone()),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut o = Map::new();
        o.insert("name".into(), json!(self.name));
        o.insert("type".into(), json!(self.type_name()));
        match &self.kind {
            ParamKind::Number { value, min, max, step } => {
                o.insert("value".into(), json!(value));
                o.insert("min".into(), json!(min));
                o.insert("max".into(), json!(max));
                o.insert("step".into(), json!(step));
            }
            ParamKind::Text { value } => {
                o.insert("value".into(), json!(value));
            }
        }
        if let Some(s) = &self.source {
            o.insert("source".into(), json!(s));
        }
        Value::Object(o)
    }

    /// Read one stored parameter leniently: a range read backwards is swapped, a value outside it
    /// clamped, junk numbers take their defaults. `Err` when it has no usable name.
    fn from_value(v: &Value) -> Result<Parameter, String> {
        let o = v.as_object().ok_or("a parameter must be an object")?;
        let name = o.get("name").and_then(Value::as_str).ok_or("a parameter without a name")?;
        check_name(name)?;
        let num = |k: &str, d: f64| o.get(k).and_then(crate::lenient::finite).filter(|f| f.abs() <= MAX_PARAM_VALUE).unwrap_or(d);
        let kind = match o.get("type").and_then(Value::as_str) {
            Some("text") => ParamKind::Text { value: o.get("value").and_then(Value::as_str).map(cut).unwrap_or_default() },
            _ => {
                let (a, b) = (num("min", 0.0), num("max", 100.0));
                let (min, max) = if a <= b { (a, b) } else { (b, a) };
                ParamKind::Number { value: num("value", min).clamp(min, max), min, max, step: num("step", 1.0).max(0.0) }
            }
        };
        let source = o.get("source").and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.len() <= 256).map(str::to_string);
        Ok(Parameter { name: name.to_string(), kind, source })
    }
}

impl Parameters {
    /// The parameter named `name` (ignoring case).
    pub fn get(&self, name: &str) -> Option<&Parameter> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Parameter> {
        self.list.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// The position of `name` (ignoring case).
    pub fn position(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Why `name` can't be a new parameter's name (or `rename`'s new name for `except`).
    pub fn check_new_name(&self, name: &str, except: Option<&str>) -> Result<(), String> {
        check_name(name)?;
        let taken = self.list.iter().any(|p| p.name.eq_ignore_ascii_case(name) && except.is_none_or(|e| !p.name.eq_ignore_ascii_case(e)));
        if taken {
            return Err(format!("there is a parameter named `{name}` already (names ignore case)"));
        }
        Ok(())
    }

    /// Read the stored form leniently: entries that don't read, repeat a name or come past
    /// [`MAX_PARAMS`] are dropped (counted in the second value).
    pub fn from_value(v: Option<&Value>) -> (Parameters, usize) {
        let entries = v.and_then(|v| v.get("params")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut out = Parameters::default();
        let mut dropped = 0;
        for e in entries {
            match Parameter::from_value(e) {
                Ok(p) if out.list.len() < MAX_PARAMS && out.get(&p.name).is_none() => out.list.push(p),
                _ => dropped += 1,
            }
        }
        (out, dropped)
    }

    pub fn to_value(&self) -> Value {
        json!({"v": PARAMS_VERSION, "params": self.list.iter().map(Parameter::to_value).collect::<Vec<_>>()})
    }

    /// The document's parameters.
    pub fn of_doc(doc: &Document) -> Parameters {
        Parameters::from_value(doc.plugin_data(PARAMS_ID)).0
    }

    /// Store these as the document's parameters (none: the plug-in data goes).
    pub fn store(&self, doc: &mut Document) {
        doc.set_plugin_data(PARAMS_ID, (!self.list.is_empty()).then(|| self.to_value()));
    }
}

/// One parameter's resolved value.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// The parameter's name as written.
    pub name: String,
    pub value: ParamValue,
    /// The value came from the active data set.
    pub from_dataset: bool,
    /// Why the data set's value wasn't taken, when it wasn't.
    pub warning: Option<String>,
}

/// The values bindings read: every parameter resolved against the active data set, and the
/// clock. Implements [`Names`] for binding expressions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    values: BTreeMap<String, Resolved>,
    clock: f64,
}

impl Scope {
    /// `params` resolved against `dataset` (the active data set's values, by variable name).
    pub fn new(params: &Parameters, dataset: Option<&BTreeMap<String, DataValue>>, known: &dyn Fn(&str) -> bool) -> Scope {
        let mut values = BTreeMap::new();
        for p in &params.list {
            let mut r = Resolved { name: p.name.clone(), value: p.own_value(), from_dataset: false, warning: None };
            if let Some(src) = &p.source {
                if !known(src) {
                    r.warning = Some(format!("no variable named `{src}`; using its own value"));
                } else if let Some(ds) = dataset {
                    match ds.get(src).map(|d| from_data(&p.kind, d)) {
                        Some(Ok(v)) => {
                            r.value = v;
                            r.from_dataset = true;
                        }
                        Some(Err(why)) => r.warning = Some(why),
                        None => r.warning = Some(format!("the data set has no value for `{src}`; using its own value")),
                    }
                }
            }
            values.insert(p.name.to_ascii_lowercase(), r);
        }
        Scope { values, clock: 0.0 }
    }

    /// The document's parameters against its active data set.
    pub fn of_doc(doc: &Document) -> Scope {
        let params = Parameters::of_doc(doc);
        if params.list.is_empty() {
            return Scope::default();
        }
        let vars = &doc.variables;
        let dataset = vars.active_dataset.as_deref().and_then(|a| vars.dataset(a)).map(|d| &d.values);
        Scope::new(&params, dataset, &|name| vars.variable(name).is_some())
    }

    /// A scope with these values and no data set (tests, previews).
    pub fn from_values(values: impl IntoIterator<Item = (String, ParamValue)>) -> Scope {
        let values = values
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), Resolved { name, value, from_dataset: false, warning: None }))
            .collect();
        Scope { values, clock: 0.0 }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The parameter `name` (any case), resolved.
    pub fn get(&self, name: &str) -> Option<&Resolved> {
        self.values.get(&name.to_ascii_lowercase())
    }

    /// Every parameter, resolved, by lowercase name.
    pub fn values(&self) -> impl Iterator<Item = &Resolved> {
        self.values.values()
    }

    /// A hash of what `names` (lowercase) resolve to (a missing name hashes as missing) and of the
    /// clock: evaluations reading the same names agree on it exactly when their values do.
    pub fn key<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> u64 {
        let mut h = DefaultHasher::new();
        self.clock.to_bits().hash(&mut h);
        for name in names {
            name.hash(&mut h);
            hash_value(self.values.get(name).map(|r| &r.value), &mut h);
        }
        h.finish()
    }

    /// A hash of every value (0 for no parameters).
    pub fn key_all(&self) -> u64 {
        if self.values.is_empty() {
            return 0;
        }
        self.key(self.values.keys().map(String::as_str))
    }
}

fn hash_value(v: Option<&ParamValue>, h: &mut DefaultHasher) {
    match v {
        None => 0u8.hash(h),
        Some(ParamValue::Number(x)) => {
            1u8.hash(h);
            x.to_bits().hash(h);
        }
        Some(ParamValue::Text(s)) => {
            2u8.hash(h);
            s.hash(h);
        }
    }
}

/// A data set's value for a parameter of `kind`.
fn from_data(kind: &ParamKind, d: &DataValue) -> Result<ParamValue, String> {
    match (kind, d) {
        (ParamKind::Number { min, max, .. }, DataValue::Text(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| ParamValue::Number(v.clamp(*min, *max)))
            .ok_or_else(|| format!("the data set's value {s:?} isn't a number; using its own value")),
        (ParamKind::Number { min, max, .. }, DataValue::Visible(b)) => Ok(ParamValue::Number(if *b { 1.0_f64 } else { 0.0 }.clamp(*min, *max))),
        (ParamKind::Text { .. }, DataValue::Text(s) | DataValue::Image(s) | DataValue::Graph(s)) => Ok(ParamValue::Text(cut(s))),
        (ParamKind::Text { .. }, DataValue::Visible(b)) => Ok(ParamValue::Text(b.to_string())),
        (ParamKind::Number { .. }, DataValue::Image(_) | DataValue::Graph(_)) => {
            Err("the data set's value isn't a number; using its own value".into())
        }
    }
}

impl Names for Scope {
    fn number(&self, name: &str) -> Option<Result<f64, String>> {
        self.values.get(name).map(|r| r.value.number())
    }

    fn clock(&self) -> f64 {
        self.clock
    }
}
