//! GPU-free, read-only model binding descriptors and verified package loaders.
//! Editing, persistence, and undo history remain owned by the editor.

pub mod model_bindings;
pub mod animation_time;
#[cfg(feature = "animated-models")]
pub mod animated_bindings;
