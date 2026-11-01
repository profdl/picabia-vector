//! Field bindings: a modifier row's number field driven by a document parameter or an expression.
//!
//! A row keeps its bindings in its own JSON, under `bind`, keyed by field, so they follow the row
//! wherever it moves (reorder, duplicate, copy and paste, object duplicate) and go with it when it
//! is removed:
//!
//! ```json
//! {"id":"m1","kind":"linearArray","count":3,"bind":{"count":{"param":"copies"},
//!  "offsetX":{"expr":"gap * 2 + value"}}}
//! ```
//!
//! # Evaluation
//! Bindings are resolved while the stack is evaluated ([`crate::evaluate_in`]), never written
//! back: the field keeps its typed value, which an expression reads as `value` and which is used
//! whenever the binding fails (a parameter that's gone or isn't a number, an expression that
//! doesn't parse or gives no number), with a warning on the field ([`BoundField::error`]).
//! Results are clamped to the field's range ([`crate::fields`]).
//!
//! An expression reads `i` and `n`, `t` (the clock: 0 for now), `value` and parameter names
//! ([`crate::expr`]). On a per-copy field ([`crate::fields::per_copy`]: `transformEach`'s and
//! `randomize`'s amounts) an expression reading `i` or `n` gives every final copy its own value
//! (`n` = all the copies), replacing the progressive ramp as a per-copy expression does. Every
//! other field takes one value: there `i` is 0 and `n` is how many copies the rows above made.
//!
//! Other binding forms (ramps, later) are kept on a round trip and ignored with a warning.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value, json};

use crate::expr::{Env, Expr};
use crate::fields::{number_field, per_copy};
use crate::model::{Modifier, ModifierKind};
use crate::params::Scope;

/// The most bindings one row keeps.
pub const MAX_BINDINGS: usize = 64;
/// The longest parameter name a binding names (bytes).
const MAX_PARAM_NAME: usize = 256;

/// What drives a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// A document parameter, by name.
    Param(String),
    /// An expression ([`crate::expr`], binding mode).
    Expr(String),
    /// A form this build doesn't evaluate (a ramp, from a newer Picabia): kept, ignored.
    Other(Value),
}

impl Binding {
    /// Read a stored binding leniently: `{"param": name}`, `{"expr": text}`, anything else kept
    /// as [`Binding::Other`].
    pub fn from_value(v: &Value) -> Binding {
        let one = v.as_object().filter(|o| o.len() == 1);
        match one.and_then(|o| o.iter().next()) {
            Some((k, Value::String(s))) if k == "param" => Binding::Param(s.clone()),
            Some((k, Value::String(s))) if k == "expr" => Binding::Expr(s.clone()),
            _ => Binding::Other(v.clone()),
        }
    }

    /// Read a binding given to `modifier.bind`: `{"param": name}` or `{"expr": text}` (a text that
    /// parses; its names are checked against the parameters by the caller).
    pub fn from_request(v: &Value) -> Result<Binding, String> {
        let o = v.as_object().ok_or("`binding` must be {\"param\": name} or {\"expr\": text}")?;
        match (o.get("param"), o.get("expr"), o.len()) {
            (Some(Value::String(p)), None, 1) if !p.trim().is_empty() && p.len() <= MAX_PARAM_NAME => Ok(Binding::Param(p.trim().to_string())),
            (None, Some(Value::String(e)), 1) => {
                let e = e.trim();
                if e.is_empty() {
                    return Err("the expression is empty".into());
                }
                if e.len() > crate::expr::MAX_EXPR_LEN {
                    return Err(format!("keep expressions under {} characters", crate::expr::MAX_EXPR_LEN));
                }
                Ok(Binding::Expr(e.to_string()))
            }
            _ if o.contains_key("ramp") => Err("ramps aren't available yet: bind a parameter or an expression".into()),
            _ => Err("`binding` must be {\"param\": name} or {\"expr\": text}".into()),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Binding::Param(p) => json!({"param": p}),
            Binding::Expr(e) => json!({"expr": e}),
            Binding::Other(v) => v.clone(),
        }
    }

    /// The parameters it reads (lowercase).
    pub fn names(&self) -> Vec<String> {
        match self {
            Binding::Param(p) => vec![p.to_ascii_lowercase()],
            Binding::Expr(e) => crate::expr::names_in(e),
            Binding::Other(_) => Vec::new(),
        }
    }

    /// Whether it reads parameter `name` (any case).
    pub fn reads(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.names().contains(&lower)
    }
}

/// Read a row's stored `bind` object (at most [`MAX_BINDINGS`] entries; the rest are dropped).
pub(crate) fn read_bindings(o: &Map<String, Value>) -> BTreeMap<String, Binding> {
    o.iter().take(MAX_BINDINGS).map(|(k, v)| (k.clone(), Binding::from_value(v))).collect()
}

/// The stored form of a row's bindings.
pub(crate) fn bindings_value(b: &BTreeMap<String, Binding>) -> Value {
    Value::Object(b.iter().map(|(k, v)| (k.clone(), v.to_value())).collect())
}

/// What a bound field resolved to in an evaluation ([`crate::Output::bound`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BoundField {
    /// The row's id.
    pub modifier: String,
    pub field: String,
    /// The value used (clamped), when the field takes one value; `None` for a per-copy
    /// expression and for a failed binding (the field's own value is used).
    pub value: Option<f64>,
    /// An expression giving every copy its own value.
    pub per_copy: bool,
    /// Why the binding wasn't used (the field's own value was), or, for a per-copy expression,
    /// for how many copies it gave no number.
    pub error: Option<String>,
}

/// A per-copy expression and how many copies it gave no number for.
#[derive(Clone, Debug)]
pub(crate) struct PerCopyExpr {
    pub field: String,
    pub expr: Expr,
    pub failures: Arc<AtomicUsize>,
}

/// The per-copy expressions of one row, by field.
#[derive(Clone, Debug, Default)]
pub(crate) struct PerCopy(pub Vec<PerCopyExpr>);

impl PerCopy {
    pub fn get(&self, field: &str) -> Option<&PerCopyExpr> {
        self.0.iter().find(|e| e.field == field)
    }
}

impl PerCopyExpr {
    /// The amount for copy `i` of `n` with the field's own value `value`, within `lo..=hi`; the
    /// own value (counted as a failure) when the expression gives no number.
    pub fn amount(&self, i: usize, n: usize, value: f64, lo: f64, hi: f64) -> f64 {
        let v = self.expr.eval(&Env::new(i, n, value));
        if v.is_finite() {
            v.clamp(lo, hi)
        } else {
            self.failures.fetch_add(1, Ordering::Relaxed);
            value
        }
    }

    pub fn failed(&self) -> usize {
        self.failures.load(Ordering::Relaxed)
    }
}

/// Row `m` with its bindings resolved.
pub(crate) struct Resolution<'a> {
    pub kind: Cow<'a, ModifierKind>,
    pub per_copy: PerCopy,
    pub fields: Vec<BoundField>,
}

/// Resolve `m`'s bindings in `scope`; `n_in` is how many copies the rows above made (`n` for the
/// fields that take one value).
pub(crate) fn resolve<'a>(m: &'a Modifier, scope: &Scope, n_in: usize) -> Resolution<'a> {
    if m.bind.is_empty() || matches!(m.kind, ModifierKind::Other { .. }) {
        return Resolution { kind: Cow::Borrowed(&m.kind), per_copy: PerCopy::default(), fields: Vec::new() };
    }
    let kind_id = m.kind.id().to_string();
    let mut params = m.kind.params();
    let mut changed = false;
    let mut per = PerCopy::default();
    let mut fields = Vec::with_capacity(m.bind.len());
    for (field, b) in &m.bind {
        let mut bf = BoundField { modifier: m.id.clone(), field: field.clone(), value: None, per_copy: false, error: None };
        let Some(spec) = number_field(&kind_id, field) else {
            bf.error = Some(format!("`{field}` isn't a number field of {}; the binding is ignored", m.label()));
            fields.push(bf);
            continue;
        };
        let Some(typed) = params.get(field).and_then(Value::as_f64) else {
            bf.error = Some(format!("`{field}` has no number to bind"));
            fields.push(bf);
            continue;
        };
        let got: Result<Option<f64>, String> = match b {
            Binding::Param(name) => match scope.get(name) {
                None => Err(format!("no parameter named `{name}`; using its own value")),
                Some(r) => r.value.number().map(Some).map_err(|why| format!("`{}` {why}; using its own value", r.name)),
            },
            Binding::Expr(text) => match Expr::parse_binding(text, scope) {
                Err(e) => Err(format!("{e}; using its own value")),
                Ok(expr) if per_copy(&kind_id, field) && expr.varies() => {
                    per.0.push(PerCopyExpr { field: field.clone(), expr, failures: Arc::new(AtomicUsize::new(0)) });
                    bf.per_copy = true;
                    Ok(None)
                }
                Ok(expr) => {
                    let v = expr.eval(&Env { i: 0.0, n: n_in.max(1) as f64, t: 0.0, value: typed });
                    if v.is_finite() { Ok(Some(v)) } else { Err("gives no number (a division by zero?); using its own value".into()) }
                }
            },
            Binding::Other(_) => Err("this kind of binding (a ramp?) isn't available yet; using its own value".into()),
        };
        match got {
            Ok(Some(v)) => {
                let v = spec.clamp(v).unwrap_or(typed);
                bf.value = Some(v);
                if let Some(n) = serde_json::Number::from_f64(v) {
                    params.insert(field.clone(), Value::Number(n));
                    changed = true;
                }
                // A bound field's own per-copy expression gives way to the binding.
                if params.remove(&crate::expr_key(field)).is_some() {
                    changed = true;
                }
            }
            Ok(None) => {
                if params.remove(&crate::expr_key(field)).is_some() {
                    changed = true;
                }
            }
            Err(e) => bf.error = Some(e),
        }
        fields.push(bf);
    }
    let kind = if changed {
        match ModifierKind::read_params(&kind_id, &params) {
            ModifierKind::Other { error, .. } => {
                // Can't happen for clamped numbers; were it to, the row keeps its own values.
                for f in fields.iter_mut().filter(|f| f.value.is_some()) {
                    f.value = None;
                    f.error = Some(format!("the bound value doesn't read ({}); using its own value", error.clone().unwrap_or_default()));
                }
                Cow::Borrowed(&m.kind)
            }
            k => Cow::Owned(k),
        }
    } else {
        Cow::Borrowed(&m.kind)
    };
    Resolution { kind, per_copy: per, fields }
}

impl Modifier {
    /// Bind field `field` to `b`, checked against `scope`: the field must be one of the kind's
    /// number fields, a parameter must exist, an expression must parse with these parameters.
    /// Replaces the field's binding. `Ok(true)` when it gives every copy its own value.
    pub fn bind_field(&mut self, field: &str, b: Binding, scope: &Scope) -> Result<bool, crate::ModError> {
        let bad = crate::ModError::Invalid;
        if let ModifierKind::Other { kind, .. } = &self.kind {
            return Err(bad(format!("can't bind a field of a `{kind}` modifier (unknown kind)")));
        }
        let kind = self.kind.id().to_string();
        if number_field(&kind, field).is_none() {
            let names: Vec<&str> = crate::fields::number_fields(&kind).iter().map(|f| f.key).collect();
            let takes = if names.is_empty() { "none".to_string() } else { names.join(", ") };
            return Err(bad(format!("`{field}` isn't a number field of {} (its number fields: {takes})", self.label())));
        }
        if !self.bind.contains_key(field) && self.bind.len() >= MAX_BINDINGS {
            return Err(bad(format!("a row keeps at most {MAX_BINDINGS} bindings")));
        }
        let varies = match &b {
            Binding::Param(name) => {
                let r = scope.get(name).ok_or_else(|| bad(format!("no parameter named `{name}` (param.add makes one)")))?;
                r.value.number().map_err(|why| bad(format!("`{}` {why}", r.name)))?;
                false
            }
            Binding::Expr(text) => Expr::parse_binding(text, scope).map_err(|e| bad(e.to_string()))?.varies(),
            Binding::Other(_) => return Err(bad("bind a parameter or an expression".into())),
        };
        self.bind.insert(field.to_string(), b);
        Ok(varies && per_copy(&kind, field))
    }

    /// Drop field `field`'s binding, or every binding with `None`; how many went.
    pub fn unbind_field(&mut self, field: Option<&str>) -> usize {
        match field {
            Some(f) => usize::from(self.bind.remove(f).is_some()),
            None => {
                let n = self.bind.len();
                self.bind.clear();
                n
            }
        }
    }
}
