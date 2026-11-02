//! The parameters of the `color` row: tlprocedural's Color and Color Each as one per-copy row
//! that shifts each copy's fill and stroke colours in OKLCH (`docs/picabia/colour.md`). Its ranges
//! and defaults, clamped when evaluating ([`crate::colour`]).

use serde::{Deserialize, Serialize};

use crate::lenient;

/// What a [`Colour`] row does to strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColourStroke {
    /// Strokes take the fill's shifts and jitter.
    #[default]
    Same,
    /// Strokes take their own shifts and jitter (`strokeHue`…).
    Own,
    /// Strokes keep their colour.
    None,
}

/// The order a [`Colour`] row's ramp and jitter count the copies in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColourOrder {
    /// The copy index across all the copies.
    #[default]
    Copies,
    /// The copy number in one array row only (`array` says which).
    Array,
    /// The copy's centre, left to right.
    LeftRight,
    /// The copy's centre, top to bottom.
    TopBottom,
    /// The copy's centre by `x + y`.
    Diagonal,
}

/// `color` (a per-copy row): shift the hue, chroma and lightness of each copy's fills and strokes,
/// by the same amount on every copy, ramped across the copies (`progressive`), at random per copy
/// (jitter), or all three. Percentages: hue 100 = a full turn, chroma 100 = +0.4 OKLCH chroma,
/// lightness 100 = +1 OKLCH lightness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Colour {
    /// Fill hue shift (−100..=100 %).
    pub hue: f64,
    /// Fill chroma shift (−100..=100 %).
    pub chroma: f64,
    /// Fill lightness shift (−100..=100 %).
    pub lightness: f64,
    #[serde(deserialize_with = "lenient::choice")]
    pub stroke: ColourStroke,
    /// Stroke shifts when `stroke` is `own` (−100..=100 %).
    pub stroke_hue: f64,
    pub stroke_chroma: f64,
    pub stroke_lightness: f64,
    /// The ramp: shifts are multiplied by `t × amount`. Off: every copy gets the full shift.
    pub progressive: bool,
    /// Ramp intensity (0..=1).
    pub amount: f64,
    /// The ramp runs the other way (the last copy keeps its colour).
    pub reverse: bool,
    #[serde(deserialize_with = "lenient::choice")]
    pub order: ColourOrder,
    /// With `order: array`: which array row (1 = the first from the top) the ramp runs along.
    #[serde(deserialize_with = "lenient::count")]
    pub array: u32,
    /// Random extra fill shift per copy, plus or minus (0..=100 %).
    pub jitter_hue: f64,
    pub jitter_chroma: f64,
    pub jitter_lightness: f64,
    /// Stroke jitter when `stroke` is `own` (0..=100 %).
    pub stroke_jitter_hue: f64,
    pub stroke_jitter_chroma: f64,
    pub stroke_jitter_lightness: f64,
    #[serde(deserialize_with = "lenient::seed")]
    pub seed: u32,
    /// Per-copy expressions for the fill shifts ([`crate::expr`]), replacing `field × ramp`.
    /// Written only when set.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub hue_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub chroma_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub lightness_expr: Option<String>,
}

impl Default for Colour {
    fn default() -> Self {
        Colour {
            hue: 10.0,
            chroma: 10.0,
            lightness: 10.0,
            stroke: ColourStroke::Same,
            stroke_hue: 0.0,
            stroke_chroma: 0.0,
            stroke_lightness: 0.0,
            progressive: true,
            amount: 1.0,
            reverse: false,
            order: ColourOrder::Copies,
            array: 1,
            jitter_hue: 0.0,
            jitter_chroma: 0.0,
            jitter_lightness: 0.0,
            stroke_jitter_hue: 0.0,
            stroke_jitter_chroma: 0.0,
            stroke_jitter_lightness: 0.0,
            seed: 42,
            hue_expr: None,
            chroma_expr: None,
            lightness_expr: None,
        }
    }
}

/// The `color` presets, by name, in menu order.
pub const COLOUR_PRESETS: &[&str] = &["rainbow", "fade", "vary"];

impl Colour {
    /// Preset `name` (`rainbow`, `fade`, `vary`; any case), or `None`.
    pub fn preset(name: &str) -> Option<Colour> {
        let base = Colour { hue: 0.0, chroma: 0.0, lightness: 0.0, ..Colour::default() };
        Some(match name.to_ascii_lowercase().as_str() {
            // One full turn of the hue across the copies.
            "rainbow" => Colour { hue: 100.0, ..base },
            // Copies fade towards white.
            "fade" => Colour { lightness: 60.0, ..base },
            // Each copy a little different.
            "vary" => Colour { progressive: false, jitter_hue: 8.0, jitter_chroma: 10.0, jitter_lightness: 10.0, ..base },
            _ => return None,
        })
    }
}
