//! The parameters of the geometry rows (`resample`, `trim`, `tidy`): rows that change the
//! object's paths instead of placing copies ([`crate::shape`] runs them). Ported from tlprocedural's
//! Subdivide and Rebuild Points (Resample), Trim Path (Trim), and Close Path and Sort Points
//! (Tidy); its ranges and defaults, clamped when evaluating ([`crate::shape::clamped`]).

use serde::{Deserialize, Serialize};

use crate::lenient;

/// How [`Resample`] rebuilds a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResampleMode {
    /// Split every segment in two, `iterations` times, by `method`.
    Subdivide,
    /// `count` points spaced evenly along each subpath, joined by straight segments.
    #[default]
    Count,
    /// A point every `spacing` points along each subpath, joined by straight segments.
    Spacing,
}

/// How [`ResampleMode::Subdivide`] splits segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubdivideMethod {
    /// A new anchor in the middle of each segment; curves keep their exact shape.
    #[default]
    Linear,
    /// A new point on the Catmull-Rom spline through the anchors between each two; straight
    /// segments.
    CatmullRom,
    /// Chaikin's corner cutting: each segment's quarter points replace its ends; straight
    /// segments (the path rounds off and shrinks a little each time).
    Chaikin,
}

/// `resample` (a geometry row): rebuild every path's anchors: subdivide each segment, or place
/// points evenly by count or by spacing along the path (by arc length).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resample {
    pub mode: ResampleMode,
    /// Subdivide: how many times each segment is split (1..=5).
    #[serde(deserialize_with = "lenient::seed")]
    pub iterations: u32,
    pub method: SubdivideMethod,
    /// Count: points per subpath (2..=1024; a closed subpath gets at least 3).
    #[serde(deserialize_with = "lenient::seed")]
    pub count: u32,
    /// Spacing: points between neighbouring points along the path (1..=5000).
    pub spacing: f64,
}

impl Default for Resample {
    fn default() -> Self {
        Resample { mode: ResampleMode::Count, iterations: 1, method: SubdivideMethod::Linear, count: 20, spacing: 24.0 }
    }
}

/// `trim` (a geometry row): keep the part of every subpath from `start` to `end`, fractions of its
/// length (0..=1), both shifted by `offset` (0..=1). A closed subpath's window wraps through its
/// start point (end before start keeps the part through it); an open one never wraps (end before
/// start keeps the part between them). The result is open; start = end keeps the whole path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Trim {
    pub start: f64,
    pub end: f64,
    pub offset: f64,
}

impl Default for Trim {
    fn default() -> Self {
        Trim { start: 0.1, end: 0.9, offset: 0.0 }
    }
}

/// How [`Tidy`] closes open subpaths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseMethod {
    /// Leave open subpaths open.
    Off,
    /// Close with a straight segment.
    #[default]
    Straight,
    /// Close with a curve that carries on both ends' directions.
    Smooth,
}

/// The order [`Tidy`] puts each subpath's anchors in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    /// Keep the order (and the curves).
    #[default]
    None,
    /// By angle around the centre of the subpath's anchors.
    Angle,
    /// By distance from that centre.
    Distance,
    /// By x, left to right.
    X,
    /// By y, top to bottom.
    Y,
    /// Shuffled by `seed`.
    Random,
}

/// `tidy` (a geometry row): close open subpaths (ends within `tolerance` become one anchor), then
/// sort each subpath's anchors (joined by straight segments once sorted) or, with `sort: none`
/// and `reverse`, reverse its direction (curves kept).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tidy {
    pub close: CloseMethod,
    /// Points (0..=500).
    pub tolerance: f64,
    pub sort: SortBy,
    /// Reverse the order.
    pub reverse: bool,
    /// For `sort: random` (0..=999999).
    #[serde(deserialize_with = "lenient::seed")]
    pub seed: u32,
}

impl Default for Tidy {
    fn default() -> Self {
        Tidy { close: CloseMethod::Straight, tolerance: 1.0, sort: SortBy::None, reverse: false, seed: 1 }
    }
}
