//! Compatibility façade; legacy example flags and consumer tests are retained.
pub use orr_package::sprite_package::authoring::*;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
