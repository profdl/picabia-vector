//! The parameters of the `displace` geometry row ([`crate::shape`] runs it). Ported from
//! tlprocedural's Noise modifier (`NoiseSettings`, `NOISE_DEFAULTS`, `NOISE_SCHEMA`): its ranges and
//! defaults, clamped when evaluating ([`crate::shape::clamped`]). Its `animated` and
//! `animationSpeed` fields are not ported (tlprocedural's processor never read them).

use serde::{Deserialize, Serialize};

use crate::lenient;

/// How [`Displace`] moves the anchors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaceMode {
    /// Each anchor moves by fractal simplex noise sampled where it sits.
    #[default]
    Spatial,
    /// Each anchor moves towards or away from the centre of the anchors, by a sine of its angle
    /// round it: `rippleCount` bumps.
    Radial,
}

/// `displace` (a geometry row): move every anchor (its handles with it) by seeded noise or a radial
/// ripple; `progressive` ramps the amount across the copies above it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Displace {
    pub mode: DisplaceMode,
    /// The most an anchor moves (points, 0..=100). Radial: percent of its distance from the
    /// centre, times a quarter.
    pub amplitude: f64,
    /// Spatial: the noise frequency per point (0.01..=1).
    pub scale: f64,
    /// Spatial: layers of noise, each twice as fine and half as strong (1..=6).
    #[serde(deserialize_with = "lenient::seed")]
    pub octaves: u32,
    /// Radial: bumps round the centre (2..=12).
    #[serde(deserialize_with = "lenient::seed")]
    pub ripple_count: u32,
    /// 0..=999999.
    #[serde(deserialize_with = "lenient::seed")]
    pub seed: u32,
    /// Ramp the amplitude across the copies of the rows above: none on the first, all on the last.
    pub progressive: bool,
    /// The last copy's share of the amplitude when progressive (0..=1).
    pub progressive_amount: f64,
    /// Ramp from the last copy to the first instead.
    pub progressive_reverse: bool,
}

impl Default for Displace {
    fn default() -> Self {
        Displace {
            mode: DisplaceMode::Spatial,
            amplitude: 10.0,
            scale: 0.05,
            octaves: 3,
            ripple_count: 4,
            seed: 42,
            progressive: true,
            progressive_amount: 1.0,
            progressive_reverse: false,
        }
    }
}
