//! Picabia stencil geometry (layer 3): turn an object's painted outlines into a laser-cut katagami
//! stencil sheet. [`gather`] reads them from a VectorCraft object (a group of traced paths, say) as
//! [`Paint`]s in document space (VectorCraft objects have no transform of their own); the rest is
//! pure geometry that computes the sheet.
//!
//! # Pipeline
//! 1. [`visible_regions`]: what each colour shows once everything above it is painted (top-down
//!    subtraction, same-colour runs merged first; booleans, not an arrangement).
//! 2. [`Selector`]: which colours are cut (`threshold` by [`luminance`], or `pick`; `invert` swaps),
//!    [`colour_infos`] for the swatch list, at the physical scale [`mm_per_unit`].
//! 3. Clean-up at physical size: specks and tiny paper holes by area, an opening by
//!    `minCutWidthMm / 2` (thin cuts), a closing by `minBridgeMm / 2` (thin paper bridges),
//!    simplification to the tolerance. The limits are widths; the radii are half of them.
//! 4. The sheet (layer bounds plus `marginMm`) and `paper = sheet − cut`.
//! 5. Islands (paper pieces the cut separates from the sheet: the holes in the cut, by contour
//!    nesting) held by tie bars to the paper around them, by a lattice of bars (`pattern`, ties for
//!    what it misses), or flagged (`gauze`); [`StencilOut::islands`], [`StencilOut::ties`].
//!
//! 6. Registration holes (hoshi) in the margin at the design's corners, [`StencilOut::hoshi`].
//! 7. Checks: what still breaks a limit (narrow paper or cuts, small cuts or holes, sharp
//!    corners, untied islands), [`StencilOut::problems`]; `limitMode: flag` skips the fixing
//!    clean-up so every violation shows.
//!
//! A repeat (a wallpaper or grid row above the stencil) makes the islands and ties repeat-aware
//! and allows a one-cell tile output: [`compute_repeat`].
//!
//! [`compute`] runs them all and returns a [`StencilOut`].
//!
//! Laser export (millimetres): [`laser_job`] turns the drawn cut into kerf-compensated, cleaned,
//! ordered sheets (split to the bed, labelled in a single-stroke font, [`text_polylines`]),
//! [`laser_svg`] writes one as an SVG for laser software, [`test_card`] makes the card that
//! measures a material's kerf and limits. [`laser_gcode`] writes a sheet as GRBL 1.1 G-code
//! (`GcodeParams`: the machine's S max, per-operation speed, power and passes; beam off at both
//! ends, travel by `G0` under `M4`), [`estimate_sec`] its run time; [`power_ladder`] and
//! [`ladder_gcode`] make the grid of squares that finds a material's power and speed. This crate
//! only writes text: it never opens a port or talks to a machine.
//!
//! [`store`] keeps computed sheets for the renderer and exports (the engine computes them), with
//! the cheap snapshot and the geometry key that say whether one is still fresh.
//!
//! Path operations: [`ops`] (booleans on `linesweeper`'s raw contours, see its docs for why not
//! VectorCraft's tidied booleans) and [`offset`] (region offset, opening, closing).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod clean;
mod colour;
mod compute;
mod gather;
mod hold;
mod hoshi;
mod index;
mod islands;
pub mod knife_gcode;
mod laser;
mod laser_bed;
mod laser_card;
mod laser_font;
mod laser_gcode;
mod laser_ladder;
mod laser_svg;
pub mod offset;
pub mod ops;
mod pattern;
mod problems;
mod repeat;
mod rings;
pub mod store;
mod ties;
mod visible;

pub use colour::{ColourInfo, Selector, colour_infos, luminance, mm_per_unit};
pub use compute::{Island, MAX_PAINTS, Stats, StencilError, StencilOut, Tile, compute, compute_repeat};
pub use gather::{Gathered, MAX_GATHER_DEPTH, MAX_GATHER_VISITS, MAX_STROKE_WIDTH, Source, gather, gather_shaped};
pub use knife_gcode::{Head, KnifeParams, knife_gcode};
pub use laser::{
    Contour, LaserInput, LaserJob, LaserParams, LaserSheet, MAX_LABEL_LINES, MAX_SHEET_MM, MAX_SHEET_POINTS, MAX_SHEETS, Polyline, laser_job,
    order_contours,
};
pub use laser_card::{BRIDGES_MM, CARD_H_MM, CARD_W_MM, CORNERS_DEG, GAUGE_MM, HOLES_MM, SLITS_MM, test_card};
pub use laser_font::{MAX_TEXT_CHARS, font_has, text_polylines};
pub use laser_gcode::{Gcode, GcodeParams, MAX_GCODE_LINES, MAX_HEADER_LINES, OpSettings, estimate_sec, laser_gcode};
pub use laser_ladder::{Ladder, LadderCell, LadderParams, MAX_LADDER_STEPS, ladder_gcode, power_ladder};
pub use laser_svg::laser_svg;
pub use offset::{MAX_OFFSET_DISTANCE, MAX_OFFSET_ELEMENTS, OFFSET_MITER_LIMIT, close, offset, open};
pub use problems::{MAX_PROBLEMS, Problem, ProblemCounts, ProblemKind};
pub use repeat::tile_cell;
pub use visible::{Region, Visible, rgb8, visible_regions};

use vectorcraft_geom::kurbo::BezPath;

/// The most outline points (after flattening) the whole design may have; shapes past it are left
/// out with a warning.
pub const MAX_POINTS: usize = 2_000_000;

/// One painted outline of the design.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    /// The filled outline in document space (picabia-craft: the layer's space), read non-zero
    /// (open subpaths closed).
    pub path: BezPath,
    /// Fill colour: sRGB 0–1 and alpha. Translucent paints count as opaque.
    pub colour: [f32; 4],
    /// Paint order, 0 = bottom. Equal orders stack in input order.
    pub order: u32,
}

#[cfg(test)]
extern crate self as vectorcraft_stencil;
#[cfg(test)]
mod clean_tests;
#[cfg(test)]
mod colour_tests;
#[cfg(test)]
mod compute_tests;
#[cfg(test)]
#[path = "../tests/support/fixture.rs"]
mod fixture;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod gather_tests;
#[cfg(test)]
mod hold_tests;
#[cfg(test)]
mod hoshi_tests;
#[cfg(test)]
mod knife_gcode_tests;
#[cfg(test)]
mod laser_card_tests;
#[cfg(test)]
mod laser_font_tests;
#[cfg(test)]
mod laser_gcode_tests;
#[cfg(test)]
mod laser_ladder_tests;
#[cfg(test)]
mod laser_tests;
#[cfg(test)]
mod offset_tests;
#[cfg(test)]
mod problems_tests;
#[cfg(test)]
mod repeat_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod visible_tests;
