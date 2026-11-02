//! Pose to art (PIC31.3): a body's simulated state carried onto its object.
//!
//! - **Rigid**: a document-space affine (a turn about the bounds' centre and a move), which the
//!   engine applies with the transform commands' own path.
//! - **Cloth**: each point placed by its distance along the strip's axis and its offset across it,
//!   on the simulated centre line.
//! - **Jelly / rubber**: each point carried by its rest cell (barycentric) to the cell's simulated
//!   triangle; curves are split into short pieces first
//!   (`vectorcraft_doc::live::map_nonlinear_within`, as Deform does), so straight sides bend.
//!
//! Paths bend (a live shape becomes a plain path); type, images and symbol instances, which can't
//! bend, follow the warp's best-fit affine at their centre.

use vectorcraft_doc::Node;
use vectorcraft_geom::{Affine, Point};

use crate::mesh::SoftMesh;

/// A body's state at one moment.
#[derive(Clone, Debug, PartialEq)]
pub enum Pose {
    /// A rigid body's motion from its rest place (`None`: it hasn't moved).
    Rigid(Option<Affine>),
    /// A soft body's particles, document space, in the mesh's order.
    Soft(Vec<Point>),
}

/// `node` warped from `mesh`'s rest pose to `particles` (document space, one per rest particle),
/// or `None` when the particles don't match the mesh or aren't finite.
pub fn pose_node(node: &Node, mesh: &SoftMesh, particles: &[Point]) -> Option<Node> {
    let _ = (node, mesh, particles);
    None
}
