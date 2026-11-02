//! The parameters of the `displace` geometry row ([`crate::shape`] runs it): one row with five
//! patterns sharing a direction, an amplitude, a size and a phase or seed. Ported from
//! tlprocedural (permission in NOTICE): the Noise modifier (`NoiseSettings`, `NOISE_DEFAULTS`,
//! `NOISE_SCHEMA`: the `spatial` and `radial` patterns), the Wave modifier (`WaveSettings`,
//! `WAVE_DEFAULTS`, `WAVE_SCHEMA`), Randomize Points (`RandomizeSettings`: `jitter`) and
//! Subdivision Growth (`SubdivisionGrowthSettings`, `SUBDIVISION_GROWTH_DEFAULTS`: `fractal`).
//! Their ranges and defaults, clamped when evaluating ([`crate::shape::clamped`]). Not ported:
//! Noise's `animated` and `animationSpeed` (its processor never read them), and Subdivision
//! Growth's repulsion, intersection clean-up, growth bias and shape influence (see
//! `docs/picabia/modifiers.md` › displace).
//!
//! Every field added after the first two patterns defaults so that a stack saved before it reads
//! and draws the same: `direction` defaults to [`DisplaceDirection::Own`], each pattern's own.

use serde::{Deserialize, Serialize};

use crate::lenient;

/// How [`Displace`] moves the anchors: its pattern. Saved as `mode` (the first two patterns'
/// field), so older stacks read unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaceMode {
    /// Smooth noise: each anchor moves by fractal simplex noise sampled where it sits.
    #[default]
    Spatial,
    /// Each anchor moves towards or away from the centre of the anchors, by a sine of its angle
    /// round it: `rippleCount` bumps.
    Radial,
    /// A periodic wave (`waveform`) across the object: `frequency` cycles from one side to the
    /// other, shifted by `phase` (tlprocedural's Wave).
    Wave,
    /// Each anchor moves by a seeded random amount between `jitterMin` and all of the amplitude
    /// (tlprocedural's Randomize Points).
    Jitter,
    /// Fractal subdivision: `steps` times, every segment is split and the new anchor pushed out
    /// by the amplitude, `decay` times less each step (tlprocedural's Subdivision Growth).
    Fractal,
}

/// Which way [`Displace`] moves each anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaceDirection {
    /// The pattern's own (tlprocedural's): smooth noise in both axes, radial and jitter away from
    /// the centre, wave across the object's width (moving in y), fractal between the radial
    /// direction and the segment's normal (`normalBias`).
    #[default]
    Own,
    /// Along the path's outward normal at the anchor (a wave runs along the outline).
    Normal,
    /// Along the copy's x axis (a wave runs down its height).
    X,
    /// Along the copy's y axis (a wave runs across its width).
    Y,
    /// Along `angle` (degrees from the copy's x axis; a wave runs across it).
    Angle,
}

/// A wave's shape (tlprocedural's `WaveType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Waveform {
    #[default]
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

/// `displace` (a geometry row): move every anchor (its handles with it) by a pattern; `fractal`
/// also adds anchors. `progressive` ramps the amount across the copies above it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Displace {
    /// The pattern.
    pub mode: DisplaceMode,
    /// Which way anchors move.
    pub direction: DisplaceDirection,
    /// With [`DisplaceDirection::Angle`]: degrees from the copy's x axis (-360..=360).
    pub angle: f64,
    /// The most an anchor moves (points, 0..=100; fractal 0..=200, tlprocedural's displacement).
    /// Radial: percent of its distance from the centre, times a quarter.
    pub amplitude: f64,
    /// Smooth noise: the noise frequency per point (0.01..=1).
    pub scale: f64,
    /// Smooth noise: layers of noise, each twice as fine and half as strong (1..=6).
    #[serde(deserialize_with = "lenient::seed")]
    pub octaves: u32,
    /// Radial: bumps round the centre (2..=12).
    #[serde(deserialize_with = "lenient::seed")]
    pub ripple_count: u32,
    /// Wave: the waveform.
    pub waveform: Waveform,
    /// Wave: cycles across the object (0.1..=10).
    pub frequency: f64,
    /// Wave: phase, degrees (0..=360).
    pub phase: f64,
    /// Jitter: the least an anchor moves, as a share of the amplitude (-1..=1; -1 moves either
    /// way). tlprocedural's minimum offset.
    pub jitter_min: f64,
    /// Fractal: subdivision steps (1..=8); each doubles the segments.
    #[serde(deserialize_with = "lenient::seed")]
    pub steps: u32,
    /// Fractal: the push's share at each step after the first (0..=0.5).
    pub decay: f64,
    /// Fractal: turns each push by a seeded angle, up to an eighth of a turn either way (0..=1;
    /// tlprocedural's `jitter`).
    pub wander: f64,
    /// Fractal: how far from the middle of a segment the new anchor may be (0..=1).
    pub edge_variance: f64,
    /// Fractal, own direction: 0 pushes away from the centre, 1 along the segment's normal.
    pub normal_bias: f64,
    /// Fractal: push in instead of out.
    pub inward: bool,
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
            direction: DisplaceDirection::Own,
            angle: 0.0,
            amplitude: 10.0,
            scale: 0.05,
            octaves: 3,
            ripple_count: 4,
            // WAVE_DEFAULTS.
            waveform: Waveform::Sine,
            frequency: 2.0,
            phase: 0.0,
            // RANDOMIZE_DEFAULTS' minDistance.
            jitter_min: 0.0,
            // SUBDIVISION_GROWTH_DEFAULTS.
            steps: 4,
            decay: 0.45,
            wander: 0.15,
            edge_variance: 0.0,
            normal_bias: 0.5,
            inward: false,
            seed: 42,
            progressive: true,
            progressive_amount: 1.0,
            progressive_reverse: false,
        }
    }
}
