//! Lenient field deserializers: counts and seeds accept any finite number (rounded, clamped), so a
//! stack written by hand or by an agent (`"count": 3.0`, `"count": -2`) still reads.

use serde::{Deserialize, Deserializer, de::Error};
use serde_json::Value;

/// A finite number from a JSON number or a numeric string (`"12"`, `" 4.5 "`).
pub(crate) fn finite(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

fn whole<'de, D: Deserializer<'de>>(d: D, max: f64) -> Result<u32, D::Error> {
    let v = Value::deserialize(d)?;
    let f = finite(&v).ok_or_else(|| D::Error::custom(format!("expected a whole number, got {v}")))?;
    // Clamped before the cast: `as` saturates anyway, but the range is the point.
    Ok(f.round().clamp(0.0, max) as u32)
}

/// A copy count: rounded and clamped to `0..=MAX_COUNT`.
pub(crate) fn count<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    whole(d, f64::from(crate::MAX_COUNT))
}

/// A seed: rounded and clamped to the `u32` range.
pub(crate) fn seed<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    whole(d, f64::from(u32::MAX))
}

/// An optional number: `null` or a blank text = none, a number (or numeric text) = it; anything
/// else is an error (the row doesn't read, as for any mistyped number).
pub(crate) fn optional_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        v => finite(&v).map(Some).ok_or_else(|| D::Error::custom(format!("expected a number or null, got {v}"))),
    }
}

/// An expression text: a string (blank = none) or a number (taken as its text); `null` = none.
pub(crate) fn expr<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) if s.len() > crate::expr::MAX_EXPR_LEN => {
            Err(D::Error::custom(format!("expressions are at most {} characters", crate::expr::MAX_EXPR_LEN)))
        }
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => Ok(Some(n.to_string())),
        v => Err(D::Error::custom(format!("expected an expression text, got {v}"))),
    }
}

/// A layer id as text: a string (trimmed) or a whole number (`12` reads as `"12"`); `null` = none.
pub(crate) fn target<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(String::new()),
        Value::String(s) if s.len() > 64 => Err(D::Error::custom("a target id is at most 64 characters")),
        Value::String(s) => Ok(s.trim().to_string()),
        Value::Number(n) if n.is_u64() => Ok(n.to_string()),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < 9.0e15 => Ok(format!("{}", f as u64)),
            _ => Err(D::Error::custom(format!("expected a layer id, got {n}"))),
        },
        v => Err(D::Error::custom(format!("expected a layer id, got {v}"))),
    }
}

/// A choice (an enum written as its camelCase name): anything it doesn't name reads as the
/// default, so a newer build's choice degrades to the default in this one.
pub(crate) fn choice<'de, D: Deserializer<'de>, T: Default + for<'a> Deserialize<'a>>(d: D) -> Result<T, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// A colour text, normalised to lowercase `#rrggbb`; `None` for anything else.
fn colour_of(v: &Value) -> Option<String> {
    v.as_str().and_then(crate::model::parse_hex_colour).map(crate::model::hex_colour)
}

fn colour_or<'de, D: Deserializer<'de>>(d: D, default: &str) -> Result<String, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(colour_of(&v).unwrap_or_else(|| default.to_string()))
}

/// A stencil's paper colour (not a colour: the default).
pub(crate) fn paper_colour<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    colour_or(d, crate::model::STENCIL_PAPER)
}

/// A stencil's paste colour (not a colour: the default).
pub(crate) fn paste_colour<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    colour_or(d, crate::model::STENCIL_PASTE)
}

/// A stencil's cloth colour (not a colour: the default).
pub(crate) fn cloth_colour<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    colour_or(d, crate::model::STENCIL_CLOTH)
}

/// A list of colour texts: entries that aren't colours are dropped, repeats kept once, at most
/// [`crate::model::MAX_CUT_COLOURS`]; one colour text reads as a list of one, anything else as
/// none.
pub(crate) fn colours<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    let items: Vec<Value> = match v {
        Value::Array(a) => a,
        s @ Value::String(_) => vec![s],
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for c in items.iter().filter_map(colour_of) {
        if out.len() >= crate::model::MAX_CUT_COLOURS {
            break;
        }
        if !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}
