//! Bounded, deterministic authoring tools for the opt-in ORAM v1 asset fixture.
//!
//! Filesystem, JSON and hashing stay here, outside the no-std asset/sim core.
//! Packages and caches provide integrity checks, not authenticity. Callers must
//! control their input/output directories during a cook (no hostile mutation).
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]

pub mod authoring;
mod pipeline;
pub use pipeline::*;

use std::{fmt, io};

#[derive(Debug)]
pub struct CookError(String);
impl CookError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CookError {}
impl From<io::Error> for CookError {
    fn from(value: io::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<serde_json::Error> for CookError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<orr_asset::AssetError> for CookError {
    fn from(value: orr_asset::AssetError) -> Self {
        Self(format!("asset validation: {value:?}"))
    }
}
pub type Result<T> = std::result::Result<T, CookError>;
