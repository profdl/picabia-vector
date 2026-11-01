//! The parameters of the `screen` geometry row (PIC20.5c), a vector halftone written for Picabia
//! ([`crate::screen`] makes the field, [`crate::shape`] runs the row). tlprocedural has no Screen;
//! nothing here is ported. Clamped when evaluating ([`crate::shape::clamped`]).

use serde::{Deserialize, Serialize};

use crate::lenient;
pub use crate::screen::SpotShape;

/// Where a [`Screen`] reads its tone (0 no ink … 1 solid ink).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToneSource {
    /// The darkness of the path's own paint (its fill, or its stroke when it has no fill): a solid
    /// colour one tone, a linear or radial gradient its colour where each point sits.
    #[default]
    Fill,
    /// `amount` percent everywhere.
    Fixed,
    /// The darkness of the image object `image` where each point sits (no ink outside it).
    Image,
}

/// `screen` (a geometry row): each path becomes the dots of a halftone screen over it, clipped to
/// its outline, filled with its paint, without a stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Screen {
    pub spot: SpotShape,
    /// Lines per inch: cells `72 / lpi` points apart (1..=300).
    pub lpi: f64,
    /// The screen's angle, degrees clockwise on screen (-360..=360).
    pub angle: f64,
    /// Samples across one cell (4..=64): higher traces the dots more closely.
    #[serde(deserialize_with = "lenient::seed")]
    pub quality: u32,
    pub tone: ToneSource,
    /// `fixed`: the tone, percent (0..=100).
    pub amount: f64,
    /// `image`: the image object's id (`"12"`); empty = not chosen.
    #[serde(deserialize_with = "lenient::target")]
    pub image: String,
    /// Ink the light parts instead of the dark ones (the tone becomes 1 − tone).
    pub invert: bool,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            spot: SpotShape::Round,
            lpi: crate::screen::DEFAULT_LPI,
            angle: 45.0,
            quality: crate::screen::DEFAULT_QUALITY,
            tone: ToneSource::Fill,
            amount: 50.0,
            image: String::new(),
            invert: false,
        }
    }
}

/// The presets `modifier.add` takes for a `screen` row (`preset`), as `(id, label)`.
pub const SCREEN_PRESETS: &[(&str, &str)] = &[("benDay", "Ben-Day Dots"), ("lineFill", "Line Fill")];

impl Screen {
    /// Preset `name` ([`SCREEN_PRESETS`]): `benDay`, round dots at 45°, 10 lpi, a fixed 30% tone
    /// (the comic-print tint); `lineFill`, lines at 45°, 12 lpi, a fixed 30% tone (a hatch).
    /// `None` for any other name.
    pub fn preset(name: &str) -> Option<Screen> {
        let fixed = |spot, lpi| Screen { spot, lpi, angle: 45.0, tone: ToneSource::Fixed, amount: 30.0, ..Screen::default() };
        match name {
            "benDay" => Some(fixed(SpotShape::Round, 10.0)),
            "lineFill" => Some(fixed(SpotShape::Line, 12.0)),
            _ => None,
        }
    }
}
