//! Reading and writing stacks: the one place a stack's JSON is parsed, with the caps.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::model::{Modifier, Stack, short};
use crate::{MAX_SOURCE_BYTES, MAX_STACK_LEN, ModError, STACK_VERSION};

/// Parse a stack from its plugin-data string.
///
/// Refused: a source over [`MAX_SOURCE_BYTES`], text that isn't JSON, JSON that isn't an object
/// with an array `stack` (a missing `stack` is an empty one), a stack over [`MAX_STACK_LEN`] and an
/// entry that isn't an object. Unknown kinds, unreadable params and unknown fields are kept (see
/// [`crate::model`]); missing or repeated modifier ids get fresh ones.
pub fn parse(src: &str) -> Result<Stack, ModError> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(ModError::TooLarge { len: src.len() });
    }
    let v: Value = serde_json::from_str(src).map_err(|e| ModError::Json(e.to_string()))?;
    stack_from_value(&v)
}

/// [`parse`] through a bounded, process-wide cache keyed by the source text: the same string
/// parses once. Thread-safe; works the same in wasm.
pub fn parse_cached(src: &str) -> Result<Arc<Stack>, ModError> {
    crate::cache::parsed(src)
}

/// Read a stack from JSON already parsed (see [`parse`] for what is refused; the size cap is the
/// caller's).
pub fn stack_from_value(v: &Value) -> Result<Stack, ModError> {
    let Value::Object(o) = v else {
        return Err(ModError::Invalid(format!("modifier data must be an object, got {}", short(v))));
    };
    let version = match o.get("v") {
        None => STACK_VERSION,
        Some(v) => v.as_u64().ok_or_else(|| ModError::Invalid(format!("`v` must be a whole number, got {}", short(v))))?,
    };
    let entries: &[Value] = match o.get("stack") {
        None | Some(Value::Null) => &[],
        Some(Value::Array(a)) => a,
        Some(v) => return Err(ModError::Invalid(format!("`stack` must be an array, got {}", short(v)))),
    };
    if entries.len() > MAX_STACK_LEN {
        return Err(ModError::TooManyModifiers { count: entries.len() });
    }
    let modifiers = entries.iter().map(Modifier::from_value).collect::<Result<Vec<_>, _>>()?;
    let extra: Map<String, Value> = o.iter().filter(|(k, _)| !matches!(k.as_str(), "v" | "stack")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut stack = Stack { version, modifiers, extra };
    stack.fix_ids();
    Ok(stack)
}

/// The stack as JSON: `{"v", "stack": [...], ...extras}`.
pub fn stack_to_value(stack: &Stack) -> Value {
    let mut o = stack.extra.clone();
    o.insert("v".into(), Value::from(stack.version));
    o.insert("stack".into(), Value::Array(stack.modifiers.iter().map(Modifier::to_value).collect()));
    Value::Object(o)
}

/// The stack as its plugin-data string.
pub fn to_json(stack: &Stack) -> String {
    stack_to_value(stack).to_string()
}
