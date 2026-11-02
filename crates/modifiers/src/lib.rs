//! Picabia modifier stacks: a live, non-destructive list of modifiers per layer.
//!
//! An object's stack lives in its Attributes as a foreign attribute ([`STACK_NS`], local name
//! [`PLUGIN_KEY`], `"picabia.modifiers"`) holding a JSON string. VectorCraft keeps foreign
//! attributes through save, copy, paste, duplicate and undo and never reads them, so plain
//! VectorCraft keeps the stack and draws the object without copies:
//!
//! ```json
//! {"v":1,"stack":[{"id":"m1","enabled":true,"kind":"linearArray","count":5,"offsetX":120}]}
//! ```
//!
//! Params are flat camelCase fields next to `kind`. Units: points for lengths, degrees for angles,
//! factors for scale (1 = 100%). Unknown kinds and unknown fields are kept on a round trip
//! ([`parse`] then [`to_json`]) and ignored when evaluating, so a newer Picabia's files degrade
//! gracefully in an older one.
//!
//! # Pieces
//! - [`model`]: [`Stack`], [`Modifier`], [`ModifierKind`] and the per-kind parameter structs.
//! - [`parse`] / [`to_json`] / [`stack_from_value`] / [`stack_to_value`]: the one place stacks are
//!   read and written, with the caps below.
//! - [`evaluate`]: a stack plus the layer's bounds (in its parent's space) to an [`Output`] of
//!   [`Instance`] transforms, canvas [`Guide`]s and [`Warning`]s.
//! - [`node_stack`], [`evaluate_node`], [`node_instances`]: cached entry points for a
//!   [`vectorcraft_doc::Node`], cheap enough to call for every layer on every frame.
//!   [`evaluate_node_in_doc`] / [`node_instances_in_doc`] also resolve `copyToShape` targets
//!   ([`DocTargets`]); the cache key includes a hash of each target's outline, so moving or editing
//!   a target re-evaluates the layers that copy onto it.
//! - [`params`] and [`bind`]: the document's parameters (numbers and texts, a data set's values
//!   when one is active) and each row's field bindings to a parameter or an expression, resolved
//!   while evaluating; [`evaluate_node_in_doc`] reads the document's, and the cache key holds the
//!   values of the parameters a stack reads ([`fields`] lists the fields a binding may drive).
//! - [`contour`] and [`screen`]: Screen's halftone core (marching squares written for Picabia, a
//!   spot-function field and image tone), run by the `screen` geometry row ([`shape`]).
//!
//! # Instance transform convention
//! VectorCraft objects keep their geometry in document space, with no transform of their own, so
//! [`Instance::transform`] maps document space to document space: a copy is the object's geometry
//! with `instance.transform` applied. An identity instance is the original object. An empty
//! [`Output::instances`] means "no active modifiers: draw the object normally". The pivot is the
//! centre of the object's geometric bounds ([`node_bounds`]).
//!
//! # Composition
//! The top row runs first (`stack[0]` first). Array stages repeat the **whole previous set as a
//! block**: for copy `k` with transform `T_k` and previous instance `S_j`, the new instance is
//! `T_k * S_j` (`T_k` applied after `S_j`, in parent space), with flat index
//! `k * n_prev + j`. Copy 0 of an array is the identity (the original counts toward `count`).
//! Per-instance stages apply a local transform `L_i` around the pivot before the instance's own
//! transform: `S_i * (pivot · L_i · pivot⁻¹)`.
//!
//! # Geometry rows
//! `resample`, `trim`, `tidy`, `deform`, `displace` and `screen` change the object's paths instead of placing copies
//! ([`shape`]): above every copy row they change the object before it is copied, below one they
//! run on each copy after its transform. [`Output::geometry`] lists them; [`Shaper`] runs them for
//! whoever builds the copies.
//!
//! # Caps
//! Sources over [`MAX_SOURCE_BYTES`] and stacks over [`MAX_STACK_LEN`] are refused by [`parse`];
//! array counts are clamped to [`MAX_COUNT`]; the instance product is checked against
//! [`MAX_INSTANCES_PER_NODE`] before allocating (the stages that fit are kept, with a warning);
//! non-finite or huge params are clamped by [`evaluate`].
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bind;
#[cfg(test)]
mod bind_tests;
mod cache;
pub mod contour;
#[cfg(test)]
mod contour_tests;
mod eval;
pub mod expr;
#[cfg(test)]
mod expr_tests;
pub mod fields;
pub mod handles;
#[cfg(test)]
mod handles_tests;
mod kinds;
#[cfg(test)]
mod kinds_tests;
mod lenient;
pub mod model;
mod model_deform;
mod model_displace;
mod model_screen;
mod model_shape;
mod node;
pub mod params;
mod parse;
pub mod screen;
#[cfg(test)]
mod screen_tests;
pub mod shape;
mod target;
#[cfg(test)]
mod target_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tone_tests;

pub use bind::{Binding, BoundField, MAX_BINDINGS};
pub use eval::{ArrayStage, Guide, Instance, Output, Repeat, StencilRow, Warning, evaluate, evaluate_in, evaluate_with, splitmix64};
pub use fields::{NumberField, number_field, number_fields, per_copy};
pub use model::{
    CircularArray, CopyMode, CopyToShape, GridArray, KINDS, LinearArray, MAX_CUT_COLOURS, MaterialLimits, Mirror, MirrorAxis, Modifier, ModifierKind,
    Randomize, Stack, Stencil, StencilColourMode, StencilLimitMode, StencilMaterial, StencilOutput, StencilPatternType, StencilPatternWhere,
    StencilPreview, StencilRepeat, StencilTieMode, TransformEach, Wallpaper, WallpaperGroup, expr_field, expr_fields, expr_key, hex_colour,
    kind_label, parse_hex_colour,
};
pub use model::{CloseMethod, Resample, ResampleMode, SortBy, SubdivideMethod, Tidy, Trim};
pub use model::{Deform, DeformAxis, DeformMode, Falloff, ShearOrigin};
pub use model::{Displace, DisplaceMode};
pub use model::{
    MAX_CORNER_ANGLE, MAX_HOSHI_DIAMETER_MM, MAX_PATTERN_SPACING_MM, MAX_TIE_WIDTH_MM, MAX_TIES_PER_ISLAND, MIN_HOSHI_DIAMETER_MM,
    MIN_PATTERN_SPACING_MM, MIN_TIE_WIDTH_MM, STENCIL_CLOTH, STENCIL_PAPER, STENCIL_PASTE,
};
pub use model::{SCREEN_PRESETS, Screen, SpotShape, ToneSource};
pub use node::{
    bound_key, evaluate_node, evaluate_node_in, evaluate_node_in_doc, evaluate_node_with, has_bindings, has_modifiers, node_bounds,
    node_bounds_in_doc, node_instances, node_instances_in_doc, node_source, node_stack, set_node_source,
};
pub use params::{ParamKind, ParamValue, Parameter, Parameters, Scope};
pub use parse::{parse, parse_cached, stack_from_value, stack_to_value, to_json};
pub use shape::{GeometryOp, GeometryRow, MAX_GEOMETRY_ANCHORS, RowAt, Shaper, shaped_copies};
pub use target::{
    DocTargets, MAX_DECODE_PIXELS, MAX_LISTED_TARGETS, MAX_TARGET_ELEMENTS, NoTargets, TargetResolver, TargetShape, ToneTarget, check_target,
    parse_target, stack_targets, valid_targets,
};

/// The local name of the foreign attribute a stack is stored under (and the key the spec names).
pub const PLUGIN_KEY: &str = "picabia.modifiers";
/// The XML namespace of that attribute; a file names it, so the attribute is ours alone.
pub const STACK_NS: &str = "https://github.com/profdl/picabia-vector/ns/modifiers";
/// The prefix the attribute is written with.
pub const STACK_PREFIX: &str = "picabia";
/// The stack format version written in `"v"`.
pub const STACK_VERSION: u64 = 1;
/// The longest stack source [`parse`] reads (256 KiB).
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
/// The most modifiers one layer's stack may hold.
pub const MAX_STACK_LEN: usize = 64;
/// The most copies one array stage makes (its `count`, rows × columns, …).
pub const MAX_COUNT: u32 = 1000;
/// The most instances one layer's stack produces (the product across stages).
pub const MAX_INSTANCES_PER_NODE: usize = 10_000;
/// Lengths (offsets, radii, gaps) are clamped to ±this many points when evaluating.
pub const MAX_LENGTH: f64 = 1.0e6;
/// Angles are clamped to ±this many degrees when evaluating.
pub const MAX_ANGLE: f64 = 1.0e6;
/// Per-copy scale factors are clamped to `0..=MAX_SCALE` when evaluating.
pub const MAX_SCALE: f64 = 100.0;

/// Why a stack couldn't be read or a modifier couldn't be changed.
#[derive(Clone, Debug, PartialEq)]
pub enum ModError {
    /// The source is longer than [`MAX_SOURCE_BYTES`].
    TooLarge { len: usize },
    /// The stack holds more than [`MAX_STACK_LEN`] modifiers.
    TooManyModifiers { count: usize },
    /// The source isn't JSON.
    Json(String),
    /// The JSON doesn't describe a stack, or a parameter value is wrong.
    Invalid(String),
}

impl std::fmt::Display for ModError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModError::TooLarge { len } => write!(f, "modifier data is {len} bytes (at most {MAX_SOURCE_BYTES})"),
            ModError::TooManyModifiers { count } => write!(f, "{count} modifiers (at most {MAX_STACK_LEN} per layer)"),
            ModError::Json(e) => write!(f, "modifier data isn't valid JSON: {e}"),
            ModError::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ModError {}
