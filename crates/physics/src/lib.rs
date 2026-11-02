//! Picabia physics: phase 1, drop-and-settle; phase 2 (PIC31), soft bodies and live simulation.
//!
//! An object's physics settings live in its Attributes as a foreign attribute ([`PHYSICS_NS`],
//! local name [`PHYSICS_KEY`], `"picabia.physics"`) holding JSON, as modifier stacks do, so save
//! and reopen, copy and paste, duplicate and undo carry them and plain VectorCraft keeps them:
//!
//! ```json
//! {"v":1,"enabled":true,"bodyType":"dynamic","colliderType":"auto","gravityScale":1.2,
//!  "restitution":0.35,"friction":0.4,"mass":1,"angularDamping":0.02,"linearDamping":0,"sensor":false}
//! ```
//!
//! The settings, their ranges and defaults are tlprocedural's Physics modifier's
//! (`docs/picabia/physics.md` lists them and the deliberate differences).
//!
//! # Pieces
//! - [`settings`]: [`Settings`], the lenient parser, the attribute.
//! - [`shape`]: an object's collision outline in document space ([`body_shape`]).
//! - [`sim`]: [`simulate`] a list of bodies; each dynamic body's rigid motion comes back as a
//!   document-space [`vectorcraft_geom::Affine`] for the engine to apply as the transform
//!   commands do.
//! - Phase 2: [`soft`] (the `soft` settings, presets, pins), [`mesh`] (an object to a soft body's
//!   rest mesh), [`pose`] (a body's state back onto its art), [`world`] (rigid and soft bodies in
//!   one live world) and [`session`] (the clock, snapshots and event log).
//!
//! # Units
//! Documents are in points; the world is in metres at [`PT_PER_M`] points per metre (an object
//! 100 pt across is 1 m), so rapier's tolerances, sleep thresholds and default density (1 kg/m²)
//! suit artboard-sized scenes. Gravity defaults to 9.81 m/s² down the page.

#![forbid(unsafe_code)]

pub mod mesh;
pub mod pose;
pub mod session;
pub mod settings;
pub mod shape;
pub mod sim;
pub mod soft;
pub mod world;

pub use settings::{BodyType, ColliderType, Settings, has_physics, node_settings, node_source, parse, set_node_settings};
pub use shape::{BodyShape, Piece, body_shape};
pub use sim::{BodyInput, BodyOutcome, SimParams, SimResult, simulate};
pub use soft::{Pin, Preset, Soft, SoftFields};

/// The attribute's local name.
pub const PHYSICS_KEY: &str = "picabia.physics";
/// The attribute's namespace (Picabia's, shared with the modifier stacks).
pub const PHYSICS_NS: &str = "https://github.com/profdl/picabia-vector/ns/modifiers";
/// The namespace prefix written in SVG.
pub const PHYSICS_PREFIX: &str = "picabia";
/// The settings format version written as `v`.
pub const SETTINGS_VERSION: u64 = 1;
/// The largest settings text read.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// Points per metre in the simulated world.
pub const PT_PER_M: f64 = 100.0;
/// The walls' thickness, points (tlprocedural's `BOUNDARY_WALL_THICKNESS`).
pub const WALL_THICKNESS: f64 = 50.0;
/// The walls' bounciness (tlprocedural's `DEFAULT_BOUNDS_RESTITUTION`).
pub const WALL_RESTITUTION: f32 = 0.3;
/// The walls' friction (tlprocedural's `DEFAULT_BOUNDS_FRICTION`).
pub const WALL_FRICTION: f32 = 0.5;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hardening;
#[cfg(test)]
mod tests_mesh;
#[cfg(test)]
mod tests_soft;
#[cfg(test)]
mod tests_soft_spike;
