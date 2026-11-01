//! Picabia stencil geometry (layer 3): turn an object's painted outlines into a laser-cut katagami
//! stencil sheet.
//!
//! Path operations: [`ops`] (booleans on `linesweeper`'s raw contours, see its docs for why not
//! VectorCraft's tidied booleans) and [`offset`] (region offset, opening, closing).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod offset;
pub mod ops;

pub use offset::{MAX_OFFSET_DISTANCE, MAX_OFFSET_ELEMENTS, OFFSET_MITER_LIMIT, close, offset, open};

#[cfg(test)]
mod offset_tests;
