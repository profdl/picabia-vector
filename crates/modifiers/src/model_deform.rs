//! The parameters of the `deform` geometry row: tlprocedural's Deform category (Taper, Twist,
//! Bend, Shear and Bulge/Pinch) as one row with a `mode`. Its ranges and defaults, clamped when
//! evaluating ([`crate::shape::clamped`]); [`crate::shape`] runs it.

use serde::{Deserialize, Serialize};

/// Which deformation [`Deform`] makes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeformMode {
    /// Scale across the axis from `startScale` at one end to `endScale` at the other.
    #[default]
    Taper,
    /// Turn the ends against each other about the centre: `-angle/2` at the start of the axis,
    /// `+angle/2` at its end.
    Twist,
    /// Turn every point about the bend point by `bendAngle` times its distance from it along the
    /// axis (as a fraction of the box).
    Bend,
    /// Slant: x moves by `shearX` times the height above the origin, y by `shearY` times the
    /// distance across.
    Shear,
    /// Push points out from the centre (positive strength) or pull them in (negative), strongest
    /// at the centre and fading to nothing at `radius`.
    Bulge,
}

/// The axis Taper, Twist and Bend run along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeformAxis {
    /// Left to right.
    #[default]
    Horizontal,
    /// Top to bottom.
    Vertical,
}

/// How Taper and Twist ease along the axis, and how Bulge fades towards its radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Falloff {
    /// Evenly.
    #[default]
    Linear,
    /// Slowly at first (`t²`; Bulge's "sharp").
    EaseIn,
    /// Quickly at first (`1 − (1 − t)²`).
    EaseOut,
    /// Slowly at both ends (smoothstep; Taper's "smooth", Twist's "ease in-out").
    Smooth,
}

/// The fixed point of a Shear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShearOrigin {
    #[default]
    Center,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

/// `deform` (a geometry row): bend the paths within their box. Above every copy row the box is
/// the object's; below one, each copy is deformed in its own box, in its own frame (the copy's
/// rotation and reflection taken out), and `progressive` ramps the effect across the array copies
/// ([`crate::Output::ramp_position`]): the least on the first, `progressiveAmount` of it on the
/// last (`progressiveReverse` the other way). Curves stay curves:
/// segments are split into short pieces whose control points are mapped, within the anchor cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Deform {
    pub mode: DeformMode,
    /// Taper, Twist and Bend.
    pub axis: DeformAxis,
    /// Taper, Twist and Bulge.
    pub falloff: Falloff,
    /// Taper: the scale across the axis at its start (0.1..=3; 1 = 100%).
    pub start_scale: f64,
    /// Taper: the scale at its end (0.1..=3).
    pub end_scale: f64,
    /// Twist: degrees between the two ends (−720..=720).
    pub angle: f64,
    /// Bend: degrees (−360..=360).
    pub bend_angle: f64,
    /// Bend: where along the axis it pivots (0..=1).
    pub bend_point: f64,
    /// Shear: x moved per point down from the origin (−2..=2).
    pub shear_x: f64,
    /// Shear: y moved per point across from the origin (−2..=2).
    pub shear_y: f64,
    pub origin: ShearOrigin,
    /// Bulge: positive pushes out, negative pinches in, as a fraction of the shape's extent
    /// (−3..=3).
    pub strength: f64,
    /// Bulge: how far it reaches, a fraction of the shape's extent (0.01..=2; 1 = 100%, reaching
    /// the farthest point).
    pub radius: f64,
    /// Ramp the effect across the array copies made above the row.
    pub progressive: bool,
    /// How much of the effect the last copy gets (0..=1).
    pub progressive_amount: f64,
    /// The first copy gets the most instead.
    pub progressive_reverse: bool,
}

impl Default for Deform {
    fn default() -> Self {
        Deform {
            mode: DeformMode::Taper,
            axis: DeformAxis::Horizontal,
            falloff: Falloff::Linear,
            start_scale: 1.0,
            end_scale: 0.2,
            angle: 90.0,
            bend_angle: 45.0,
            bend_point: 0.5,
            shear_x: 0.3,
            shear_y: 0.0,
            origin: ShearOrigin::Center,
            strength: 0.35,
            radius: 1.0,
            progressive: true,
            progressive_amount: 1.0,
            progressive_reverse: false,
        }
    }
}
