//! Soft bodies from objects (PIC31.2): an object's outline to a rest mesh, the particles and cells
//! (or strip) rapier simulates, in document space.
//!
//! - **Cloth**: the outline's long axis (its principal axis, so it turns with the object) is the
//!   strip, its extent across the thickness: a line of particles along the centre, `spacing` apart.
//! - **Jelly / rubber**: the closed outlines (resampled to `spacing`) filled with triangle cells;
//!   holes stay holes.
//!
//! Caps: [`MAX_PARTICLES_PER_BODY`] and [`MAX_CELLS_PER_BODY`] per object (a body that would go
//! over gets a coarser spacing, with a note); the run's total is the caller's budget.

use vectorcraft_geom::{Point, Vec2};

use crate::shape::BodyShape;
use crate::soft::Soft;

/// The most particles one soft body has (more: a coarser spacing, with a note).
pub const MAX_PARTICLES_PER_BODY: usize = 1_200;
/// The most cells one soft body has (more: a coarser spacing, with a note).
pub const MAX_CELLS_PER_BODY: usize = 2_400;
/// The most particles in one run (soft bodies past it are left out, with a note).
pub const MAX_PARTICLES_PER_RUN: usize = 12_000;

/// A cloth strip's frame, document space.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// The centre line's start (the end where `along` is 0).
    pub origin: Point,
    /// Unit vector along the strip.
    pub axis: Vec2,
    /// The strip's length along `axis`, points.
    pub length: f64,
    /// Its extent across, points.
    pub thickness: f64,
}

/// How the rest mesh is laid out.
#[derive(Clone, Debug, PartialEq)]
pub enum MeshKind {
    /// Cloth: particles along the strip's centre line, in order.
    Strip(Strip),
    /// Jelly, rubber: particles filling the outline, joined by triangle cells.
    Cells,
}

/// A soft body's rest mesh, document space.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftMesh {
    pub kind: MeshKind,
    /// The particles' rest positions.
    pub rest: Vec<Point>,
    /// Triangle cells (indices into `rest`): empty for a strip.
    pub cells: Vec<[u32; 3]>,
    /// The strip's segments, or the cells' boundary.
    pub segments: Vec<[u32; 2]>,
    /// Particle radius, points (the contact skin; half the strip's thickness for cloth).
    pub radius: f64,
    /// The spacing used, points (coarser than asked when a cap was reached).
    pub spacing: f64,
    pub notes: Vec<String>,
}

/// The rest mesh of an object whose outline is `shape`, with `soft`'s preset and fields, within
/// `budget` particles (at most [`MAX_PARTICLES_PER_BODY`]); or why it has none.
pub fn soft_mesh(shape: &BodyShape, soft: &Soft, budget: usize) -> Result<SoftMesh, String> {
    let _ = (shape, soft, budget);
    Err("soft bodies are not built yet (PIC31.2)".into())
}
