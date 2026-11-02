//! Explicit, bounded unsigned RAW16 terrain authoring and no-replace publication.
//! The existing conversion API is re-exported from the optional presentation
//! module so Editor callers need not depend on this CLI/acceptance crate.
#![deny(clippy::float_arithmetic)]
pub use orr_terrain_view::raw16::{import_raw16, Endian, Error, Recipe, Result, MAX_RAW_BYTES};
pub mod publish;
